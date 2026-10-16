# Tasks: Durable workflow primitives and practical SDK examples

**Input**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`.
**Scope**: T014-001 and US01 document/deliver current behavior. Architecture is accepted in T014-002; the bounded switch definition and reducer are implemented, while its fault and all-language runtime gates remain open. Every task implementing another primitive
or claiming its release evidence remains unchecked until its named tests pass. Tests precede implementation within each
pending slice. IDs are feature-scoped per the constitution, overriding generic template numbering.
T014-045 was added by the validation of 2026-10-14; it sits in Phase 2 by dependency, so the IDs
there are not in numeric order. Existing IDs were kept so that references stay valid.

## Phase 1 — Setup and architecture

- [x] T014-001 Specify the bounded primitives, dependencies, data model, contract proposal, fault matrix and quickstart in `specs/014-workflow-primitives/`; preserve the distinction between current examples and future runtime semantics. Refs: FR-014-001–017, NFR-014-001–004, SC-014-001–006.
  _Evidence: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/README.md`, `contracts/conformance.md`, `quickstart.md`, this task list and `checklists/requirements.md`; no runtime guarantee inferred._
- [x] T014-002 Accept structured instances/continuation as ADR-0030 and the separate declared-operation value-disclosure policy as ADR-0031; supersede the initial separate-revision allocation with additive v1 in ADR-0032; update dependent 001/002/003/004/005/006/007/008/009/010/011 feature docs before their contract changes. Record additive v1/worker 1.5, effective evaluator limits, iteration compensation, role grants, bounded product defaults, derived-instance admission reserve, child-link lock order and explicit release gates. Refs: FR-014-003,005,007–008,010–011,014–017, NFR-014-004.
  _Evidence: `docs/adr/ADR-0030-structured-instances-and-continuation-chains.md`, `ADR-0031-declared-operation-result-disclosure.md`, and superseding `ADR-0032-additive-workflow-ir.md`, aligned feature design/contracts and dependent spec/plan sections. Documentation consistency verified with relative Markdown target checks (anchors excluded) and `git diff --check`; no runtime implementation, benchmark or security result inferred._
- [x] T014-003 Register future contract/integration/fault/e2e targets in `tests/Cargo.toml` and create real-platform harnesses in `tests/{contract,integration,fault,e2e}/workflow-primitives/main.rs` with deterministic barriers; reject skipped infrastructure as passing evidence. Refs: NFR-014-001, SC-014-002–005.
  _Evidence: contract, PostgreSQL integration and fault targets plus `e2e_workflow_primitives` run against real infrastructure. The E2E test publishes through HTTP, starts positive/default/error cases and reads committed graphs without sleeps; it passed 1/1. This harness registration does not close the remaining primitive-specific release gates._

## Phase 2 — Runtime foundation (blocks US02–US07, not current-API US01)

- [ ] T014-004 Write failing additive-compatibility, identity, limit and public-operation fixtures in `tests/contract/workflow-primitives/` and shared cases WP-02–14 in `tests/contract/sdk-conformance/scenarios/`; quote/assert the data-model ceilings including `nodeId` 1..64, definition nodes 10000, depth 1..8 and 1024-byte effective expressions, 64-element list literals and 4096-element constructed lists; reject unsupported object literals and test 10,000-item reference materialization separately. Refs: FR-014-003,015–016, NFR-014-004.
- [ ] T014-005 Add typed scope/instance identity and old-root replay in `crates/domain/src/execution/instance.rs`, `state.rs`, `event.rs`, `command.rs`, `error.rs` and tests in `crates/domain/tests/`; enforce “indices are zero-based integers in 0..9999 and path depth is at most 8”, unique `(scopeInstanceId, iterationIndex)`, independent attempt/fence and acyclic/nonrecursive closed bodies. Refs: FR-014-003,014, NFR-014-001.
- [ ] T014-006 Add the approved compatible IR extensions and operation schemas in `specs/001-platform-foundation/contracts/{workflow.schema.json,openapi.yaml,worker.proto,events.asyncapi.yaml}`, the corresponding `crates/infra-ir/src/` mapping and `crates/api-grpc/src/` negotiation; retain IR v1 and use independent worker minor 1.5, withhold scoped tasks from old workers and retain every accepted old form; add no `Error.code` value, only the `details.reason` values of `contracts/README.md` with their remediation rows in `crates/api-http/src/error.rs`. Refs: FR-014-003,016–017, SC-014-005.
- [ ] T014-007 Implement additive tenant-scoped migrations in `crates/infra-postgres/migrations/`, transactional primitive ports in `crates/application/src/primitives.rs` and adapter in `crates/infra-postgres/src/primitives.rs`; cover root backfill, stable lock order, CAS/fence, cancellation-safe begin, atomic quota/audit/outbox and rollback in `tests/integration/workflow-primitives/transactions.rs`. Refs: FR-014-014, NFR-014-001, SC-014-003.
- [ ] T014-008 Extend `crates/application/src/{authz.rs,quotas.rs}` and `crates/infra-postgres/src/{quotas.rs,content_usage.rs}` for named operations, approval/chain scopes, cross-run tenant slot admission and retention; enforce “state and operation result values are at most 256 KiB each”, 4 MiB retained/run, 8 MiB context and “maxInstances is 1..100000” with top-level limits defaults 100000/8 and a derived-instance drain reserve; test policy reductions and denied cross-tenant references in `tests/integration/workflow-primitives/security.rs`. Refs: FR-014-015, NFR-014-002–003, SC-014-004,006.
- [ ] T014-009 Add primitive span/metric/error/audit vocabulary in `crates/telemetry/src/{metrics.rs,span.rs}`, `crates/application/src/error.rs` and `crates/domain/src/audit.rs`; test bounded metric labels, trace/request IDs, failed-audit rejection and secret-marker redaction in `tests/integration/workflow-primitives/observability.rs`. Refs: FR-014-017, SC-014-004.
- [ ] T014-045 Project scoped instances, selected routes and chain links into the journal-derived runtime graph (`/runs/{runId}/graph`, `/graph/changes`; ADR-0016) and the telemetry projector (ADR-0017) in `crates/application/src/` and `crates/infra-postgres/src/`; make the Studio and the projector show unknown node kinds as unsupported instead of failing, with tests in `tests/integration/workflow-primitives/projection.rs`. Full Studio authoring remains a separately approved 007 delivery. Refs: FR-014-017, NFR-014-004.

## Phase 3 — US01: realistic examples in five languages (P1, current delivery)

**Independent test**: The shared current-v1 documents execute five business cases in every SDK:
amount 100 accepted, amount -1 rejected, fulfillment, reminder and reminder NOTIFICATION_REJECTED.
This phase reuses existing foundations and does not depend on unchecked T014-002–009.

- [x] T014-010 [US01] Define shared current-v1 workflows in `examples/order-processing/workflows/` and real-engine assertions in `tests/e2e/developer-experience/order_processing.rs` for canonical definitions, activities, branch/timer ordering, terminal errors, traces and invalid configuration. Refs: FR-014-001–002, SC-014-001.
  _Evidence: shared workflow documents and the all-five matrix recorded in `specs/014-workflow-primitives/validation-report.md`; 25 workflow runs and 10 usage checks passed._
- [x] T014-011 [P] [US01] Implement runnable current-API clients/workers and prepare/run scripts in `examples/order-processing/typescript/` and `examples/order-processing/go/`, using environment credentials, bounded client waiting and stable effect-key guidance. Refs: FR-014-001–002, SC-014-001.
  _Evidence: TypeScript/Go execution in the same successful matrix, with clippy for its Rust harness, recorded in `specs/014-workflow-primitives/validation-report.md`._
- [x] T014-012 [P] [US01] Implement matching clients/workers and prepare/run scripts in `examples/order-processing/python/`, `examples/order-processing/dotnet/` and `examples/order-processing/java/`; retain common business outcomes and errors without language-specific orchestration. Refs: FR-014-001–002, SC-014-001.
  _Evidence: Python/.NET/Java execution in the all-five successful matrix recorded in `specs/014-workflow-primitives/validation-report.md`._
- [x] T014-013 [US01] Document prerequisites, publishing/bindings, all five cases, simulated effects and available/proposed boundaries in `examples/order-processing/README.md`, language READMEs and feature 012 docs; run the shared matrix and record exact commands/gaps in `specs/014-workflow-primitives/validation-report.md`. Refs: FR-014-001–002, SC-014-001.
  _Evidence: `cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::` passed 1 aggregate test (25 runs + 10 usage checks); current delivery report owns exact environment and additional checks._

## Phase 4 — US02: bounded collections (P1, restricted public slice)

The public parser accepts a root chain of `forEach` nodes with an Activity→Return body and an
explicit tenant iteration policy. PostgreSQL, fault and five-SDK scoped-worker identity tests
cover that slice, including transactional payload retention and storage-quota drain. A public
`map` result expression now commits with source-checkpoint proofs, and its runtime scenario passed
all five SDKs. The tasks below remain open for broader bodies, scoped compensation/reconciliation
and the complete shared collection conformance matrix; evidence is in `validation-report.md`.

**Independent test**: 17 items, concurrency 3, reverse completion, restart, empty/oversized input and
permanent failure; one ordered result per index, no fourth active iteration and correct saga drain.

- [ ] T014-014 [P] [US02] Add failing collection/model/tenant contention cases in `crates/domain/tests/primitive_collections.rs` and `tests/integration/workflow-primitives/collections.rs`; assert “maxItems and maxIterations are integers in 1..10000”, “maxConcurrency is 1..256”, snapshot immutability and shared run/tenant budgets, descendant admission with low quotas and derived-instance reservations that survive policy reduction. Refs: FR-014-003–005,015, NFR-014-003.
- [ ] T014-015 [US02] Implement typed collection entry/admission/ordered aggregation/fail-fast drain in `crates/domain/src/execution/{instance.rs,plan.rs,state.rs,event.rs}` and definition rules in `crates/domain/src/workflow.rs`; charge each admitted logical instance once and release slots only after terminal/reconciled drain. Refs: FR-014-004–005,014.
- [ ] T014-016 [US02] Wire captured arrays/blob refs and transactional collection progress in `crates/application/src/primitives.rs` and `crates/infra-postgres/src/primitives.rs`; add equivalent forEach/map builders to `sdks/{typescript,go,python,dotnet,java}/` and reviewed WP-03 outputs in `tests/contract/sdk-conformance/expected/`. Refs: FR-014-004,016–017.
- [ ] T014-017 [US02] Prove crash/duplicate/stale-fence recovery, lost broker reconciliation, cancellation and unknown/failed compensation in `tests/fault/workflow-primitives/collections.rs`; verify FR-003-002 compensation order (reverse topological within an iteration, no order between collection iterations) and all-five WP-03/04 conformance. Refs: FR-014-005,014, NFR-014-001, SC-014-002–004.

## Phase 5 — US03: durable loops (P1, restricted public slice enabled)

Entry, checkpoint, delay and terminal drain have PostgreSQL restart, timer-race, rollback and
projection evidence. Loop carry/result bytes now commit in the progress transaction after the
scheduler fence, replay, revision and quota checks. A restricted public activity/return body passed
PostgreSQL worker, restart and rollback tests, and one `while`/`repeatUntil` scenario passed the
five-SDK runtime conformance. Broader bodies, delay/error routing, nested budgets and the full fault
matrix remain open.

**Independent test**: False pre-test, true post-test, three durable carry checkpoints, interruption
after second commit, explicit loop-delay timer and limit/deadline exhaustion.

- [ ] T014-018 [P] [US03] Write failing pre/post-test/carry/budget fixtures in `crates/domain/tests/primitive_loops.rs` and `tests/contract/workflow-primitives/loops.rs`; enforce “timeoutMs is 1..31622400000”, delay in the same range, required maxIterations, body-return boundaries and no workflow end/continuation inside bodies. Refs: FR-014-003,006,015.
- [ ] T014-019 [US03] Implement recorded condition/carry checkpoints and loop state in `crates/domain/src/execution/{instance.rs,plan.rs,state.rs,event.rs}`; check exhaustion before scheduling another delay and preserve outer cancellation/compensation outcomes. Refs: FR-014-006,014.
- [ ] T014-020 [US03] Implement atomic checkpoint/timer/next-instance storage in `crates/infra-postgres/src/primitives.rs`, loop ports in `crates/application/src/primitives.rs` and while/repeatUntil builders in `sdks/{typescript,go,python,dotnet,java}/`; add WP-05 reviewed observations. Refs: FR-014-006,016–017.
- [ ] T014-021 [US03] Test checkpoint restart, duplicate completion, timeout-vs-signal/update/cancel, nested-budget contention and compensation of completed iterations in reverse iteration order in `tests/fault/workflow-primitives/loops.rs`; run five-SDK WP-05 parity and document the zero-worker delay scenario in `specs/014-workflow-primitives/quickstart.md`. Refs: FR-014-006,014–015, SC-014-002–004,006.

## Phase 6 — US04: authorized human approval (P1, restricted root slice enabled)

The reviewed root approval shape has public scheduler, HTTP, PostgreSQL, fault and five-SDK
decision evidence in `validation-report.md`. Approvals inside structured bodies, combinations
with declared state and chain-addressed execution remain closed.

**Independent test**: Pending approval survives restart, unauthorized identity cannot decide, and
valid response/expiry/cancel races produce one persisted outcome and matching route.

- [ ] T014-022 [P] [US04] Write failing approval contract/security/race tests in `tests/contract/workflow-primitives/approvals.rs` and `tests/integration/workflow-primitives/approvals.rs`; enforce “Between 1 and 32 total subjects/roles” with roles limited to `admin`/`developer`/`operator`, default exclusion of the run's `startedBy`, `FORBIDDEN`/`INITIATOR_CANNOT_DECIDE` for it, “decisionKey is 1..128 permitted ASCII characters” and optional reasonCode 1..64 with no free text. Refs: FR-014-007–008, NFR-014-002.
- [ ] T014-023 [US04] Implement approval request/receipt lifecycle and first-commit outcome in `crates/domain/src/execution/approval.rs` and `crates/application/src/primitives.rs`, using existing identity/name validators for bounded values and no signal-to-decision conversion. Refs: FR-014-007–008,014.
- [ ] T014-024 [US04] Add transactional request/expiry/decision/audit storage in `crates/infra-postgres/src/primitives.rs`, authorized operations in `crates/api-http/src/{routes/primitives.rs,dto_primitives.rs}` and approval builders/client calls in `sdks/{typescript,go,python,dotnet,java}/`; emit declared telemetry/errors. Refs: FR-014-007–008,016–017.
- [ ] T014-025 [US04] Prove both race commit orders, restart, duplicate/different-body receipts, RBAC revocation, raw-signal denial, audit rollback and cross-tenant opacity in `tests/fault/workflow-primitives/approvals.rs`; run all-five WP-06/14 cases. Refs: FR-014-008,014, NFR-014-001–002, SC-014-002–004.

## Phase 7 — US05: multiway switch (P2, enabled)

**Independent test**: Overlapping true cases select the first, no match selects default, errors fail
without fallthrough and replay preserves the recorded case.

- [x] T014-026 [P] [US05] Add failing switch definition/evaluation fixtures in `crates/domain/tests/{switch.rs,switch_execution.rs}` and `tests/contract/workflow-primitives/revision.rs`; enforce “1..32 ordered cases; case ids unique within switch; mandatory default” and the existing expression bound. Refs: FR-014-003,009.
  _Evidence: domain switch tests, schema contract, codec validation and the real PostgreSQL first-match/default/error tests in `tests/integration/workflow-primitives/switch_storage.rs`._
- [x] T014-027 [US05] Implement switch validation, typed decision and recorded routing in `crates/domain/src/workflow.rs`, `crates/domain/src/execution/{plan.rs,state.rs,event.rs}` and `crates/infra-ir/src/`; retain binary condition semantics. Refs: FR-014-009,014.
  _Evidence: selected local case or default is journaled and projected atomically; domain replay, IR codec and real PostgreSQL integration tests pass. This does not close fault or all-language runtime parity._
- [x] T014-028 [US05] Add all-five switch builders in `sdks/{typescript,go,python,dotnet,java}/`, run WP-07 golden parity and persist/restart/fault evidence in `tests/fault/workflow-primitives/switch.rs`; document typed predicate failure and telemetry. Refs: FR-014-009,016–017, SC-014-002–003.
  _Evidence: the enabled `switch-builders` scenario and manually reviewed goldens pass in all five SDKs for first match, default, predicate error and non-selected nodes; 3 real-PostgreSQL fault tests pass for lost response/duplicate replay, restart with an undelivered hint and transactional rollback/retry. Typed `EXPRESSION_ERROR`, trace span and bounded metric evidence is recorded in the switch validation report._

## Phase 8 — US06: declared Query/Update (P2, run-addressed slice enabled)

**Independent test**: Query reports one committed revision; concurrent expected-state-revision updates
produce one apply/one rejection; lost-response replay returns the original receipt with one mutation.
**Release gate**: ADR-0031 accepts the disclosure policy. The run-addressed slice passed its
publication, security/audit/result-bound, transaction-fault and all-five runtime conformance gates.
Chain Query/Update have a focused HTTP/transaction slice, including head CAS and receipt replay
across rollover. A direct public continuation transfers the declared snapshot atomically. Public
Query and Update after handoff, exact Update replay and a stale expected-head Query pass all-five
SDK conformance. The remaining fault and race matrix stays open.

- [ ] T014-029 [P] [US06] Write failing state/operation schemas and malicious-handler fixtures in `tests/contract/workflow-primitives/operations.rs`; enforce “At most 32 queries and 32 updates”, names 1..128, bounded state/closed result schemas, explicit `disclosure: declared-business-result-v1`, no I/O/callback/wait and “expected state revision is an optional nonnegative integer”. Refs: FR-014-010–011,015, NFR-014-002.
- [ ] T014-030 [US06] Model schema-checked initial state, snapshot query, atomic validator/reducer/result and APPLIED/REJECTED receipts in `crates/domain/src/execution/operations.rs` and `crates/application/src/primitives.rs`; preserve “State revision starts at 0 and increments exactly once per successful Update”; an Update creates no task, timer or subscription. Refs: FR-014-010–011,014.
- [ ] T014-031 [US06] Implement consistent snapshot reads/access audits and serialized update storage in `crates/infra-postgres/src/primitives.rs`, and namespace operation RBAC/endpoints with no-store responses in `crates/api-http/src/{routes/primitives.rs,dto_primitives.rs}`; enforce updateKey 1..128 and unique `(tenantId, namespaceId, addressScope, targetId, updateKey)`, canonical hash, current auth on replay and durable business rejection. Refs: FR-014-010–011,015,017.
- [ ] T014-032 [US06] Add typed state/operation declarations and Query/Update calls in `sdks/{typescript,go,python,dotnet,java}/`; execute shared WP-08/09 definitions/results/errors in `tests/contract/sdk-conformance/`, including decimals/UTC and caller cancellation without hidden reapplication. Refs: FR-014-010–011,016, NFR-014-004.
- [ ] T014-033 [US06] Prove concurrent updates vs worker/timer/signal, lost reply, crash rollback, rejection replay, non-mutable state rules, secret redaction and revoked receipt access in `tests/fault/workflow-primitives/operations.rs`; assert Query never alters workflow journal/scheduling. Refs: FR-014-010–011,014, NFR-014-001–002, SC-014-002–004.

## Phase 9 — US07: continuation chains (P3, restricted public slice)

The published input-only continuation path and chain inspect/signal/cancel are covered by focused
PostgreSQL, fault and five-SDK checks. The direct stateful continuation now has public PostgreSQL
handoff evidence. Chain Query/Update have seeded-state transaction/HTTP checks. Public Query and
Update on a stateful successor, exact Update replay and stale expected-head Query pass all-five
SDK conformance. Child continuation and the rest of the shared fault matrix remain open.
Explicit chain-completion waits now run in all five SDKs against a continued predecessor,
non-final timeout and final cancelled successor; the broader WP-10/11 matrix remains open.
The shared `chain-final-success` scenario also reaches `SUCCEEDED` after two segments in all five
SDKs. An invalid continuation input now has a durable `EXPRESSION_ERROR` regression test.
The restricted conditional graph can now transfer the exact declared state and finish normally;
the `chain-state-final-success` scenario passes all five SDKs and a PostgreSQL integration test.
An activity followed by a signal-controlled `waitEvent` can now continue after the signal.
The shared `chain-commands` scenario passes all five SDKs for signal replay, head fencing,
successor cancellation and tenant opacity. It also verifies replay after rollover, key-body
conflict and rejection of a signal to a head without a matching wait. A direct `waitEvent`
entrypoint remains closed. A real PostgreSQL fault test now proves rollback and one retry when
the outbox insert fails after a signal-controlled wait has released the continuation boundary.

**Independent test**: Three segments have one current head and unique successors; crash around each
handoff and race commands; segment wait and chain wait differ intentionally; child parent waits FINAL.

- [ ] T014-034 [P] [US07] Write failing chain/handoff/parent contracts in `tests/contract/workflow-primitives/chains.rs` and `tests/integration/workflow-primitives/chains.rs`; enforce “maxSegments is 1..10000, including the initial run”, required original timeout, segmentIndex 0..9999, unique predecessor/successor, explicit state/input transfer and the 1,024-run tree budget for successors of a continuing child. Refs: FR-014-012–013,015.
- [ ] T014-035 [US07] Model chain head/revision/cancel latch, FINAL/CONTINUED completion kind, quiescence and same-version transfer in `crates/domain/src/execution/chain.rs`, `state.rs`, `plan.rs` and `event.rs`; reject handoff with open work/messages/compensation or exhausted limits. Refs: FR-014-012–013,015.
- [ ] T014-036 [US07] Implement one handoff transaction with old completion, successor start/state/tasks, parent lineage, head, charges, audits and outbox in `crates/infra-postgres/src/primitives.rs`; preserve original deadline and retain chain/receipt/blob references through cleanup in `crates/application/src/maintenance.rs`. Refs: FR-014-012,014–015, NFR-014-001.
- [ ] T014-037 [US07] Implement chain inspect/query/update/signal/cancel in `crates/application/src/primitives.rs` and `crates/api-http/src/{routes/primitives.rs,dto_primitives.rs}`; serialize chain-before-run resolution and durable parent child-instance link cancellation/finalization, expectedRunId, stable chain keys (1..128), the existing non-buffered signal rule on the resolved head (`CONFLICT`/`STATE_CONFLICT` when it is not waiting) and exact-run non-forwarding. Refs: FR-014-013–014,017.
- [ ] T014-038 [US07] Add Continue-As-New builders, chain clients and explicit chain-completion waits in `sdks/{typescript,go,python,dotnet,java}/`; retain existing segment waits and match WP-10/11 normalized observations in `tests/contract/sdk-conformance/expected/`. Refs: FR-014-012–013,016, NFR-014-004.
- [ ] T014-039 [US07] Prove handoff crash/duplicate/lost delivery, command races, child final outcome once, parent cancellation, RERUN_AS_NEW of a segment starting an unrelated run and old-client status compatibility in `tests/fault/workflow-primitives/chains.rs`; verify no continuation bypass of chain/tenant quotas or version pinning. Refs: FR-014-012–016, SC-014-002–005.

## Final phase — Cross-cutting release gates (pending)

- [ ] T014-040 Execute the complete barrier/race/duplicate/lost-broker/failed-compensation/restart and tenant/secret matrix in `tests/fault/workflow-primitives/` and `tests/integration/workflow-primitives/`; retain seeds, commands, failed/skipped results and independently inspected persistence in `specs/014-workflow-primitives/validation-report.md`. Refs: FR-014-014–015, NFR-014-001–002, SC-014-002–004.
- [ ] T014-041 Execute all-five WP-02–14 plus active-old-run/old-worker-minor upgrades in `tests/contract/sdk-conformance/` and `tests/integration/workflow-primitives/compatibility.rs`; publish a reviewed version/capability matrix in `docs/sdk-compatibility.md` without auto-blessing goldens. Refs: FR-014-016, NFR-014-004, SC-014-002,005.
  _Current evidence: all 36 active shared scenarios pass across five SDKs with explicit 1.4
  historical worker fixtures and 1.5 primitive fixtures. The missing WP cases and upgrade matrix
  keep this task open; see the validation report._
- [ ] T014-042 Measure plan workloads with `tests/performance/workflow-primitives/` and publish hardware/configuration/raw summaries/failures in `tests/performance/reports/workflow-primitives.md`; prove cross-run tenant concurrency bounds and zero dedicated workers for waits before accepting production ceilings. Refs: FR-014-015, NFR-014-003, SC-014-006.
- [ ] T014-043 Run formatting, Clippy, locked workspace, architecture, canonical contract compilation, Compose smoke and dependency advisory gates; update primitive operator guidance in `docs/runbooks/workflow-primitives.md`, schema/API reference and `specs/014-workflow-primitives/quickstart.md` with happy/failure demos and compatible-reader rollback. Refs: FR-014-017, NFR-014-001–004, SC-014-002–006.
- [ ] T014-044 Update `docs/traceability.md`, dependent feature docs and `specs/014-workflow-primitives/{tasks.md,validation-report.md}` with requirement→test→contract→metric/log→demo evidence; enable only fully proven primitives and leave missing tests unchecked. Refs: FR-014-003–017, SC-014-002–006.

## Dependencies and execution order

- Current delivery: T014-001 → T014-010 → T014-011/T014-012 → T014-013. It uses existing engine APIs;
  its evidence predates ADR acceptance or implement future primitives as a shortcut.
- Runtime: T014-002 → T014-003/004 → T014-005/006 → T014-007 → T014-008/009/045. Contract fixtures
  precede implementation; identity and capability contracts settle together before persistence.
- US02/03 depend on the full foundation. US04/05/06 also depend on foundation but not on collections
  or each other semantically. US07 additionally depends on US06's state/receipt serialization.
- Story runtime code shares reducer/store/SDK files: serialize those edits even when story tests can
  be written in parallel. T014-040–044 run after the desired story slices; a full feature release
  requires all slices. Each slice first runs its own listed tests and existing affected regressions.
- T014-041 follows per-story SDK work; T014-042 follows correctness and does not excuse missing faults.
  T014-044 closes only evidence actually obtained, never unimplemented promises.

## Parallel opportunities by story

`[P]` means disjoint files after the stated prerequisite, not permission for conflicting edits.
US01 language tasks T014-011/012 can run together after T014-010. US02/US03 test tasks T014-014/018,
US04 T014-022, US05 T014-026 and US06 T014-029 can run independently after foundation contracts;
US07 T014-034 follows US06 semantics. Within those tasks domain and adapter test files are distinct.
Implementation tasks touching shared reducer/store/SDK files remain sequential unless ownership is
explicitly split by file. Do not infer that all languages or stories should be delegated automatically.

## Requirement and success-criterion coverage

| Requirement / criterion | Tasks |
| --- | --- |
| FR-014-001–002, SC-014-001 | T014-010–013 |
| FR-014-003 | T014-002,004–006,014,018,026 |
| FR-014-004–005 | T014-014–017 |
| FR-014-006 | T014-018–021 |
| FR-014-007–008 | T014-002,022–025 |
| FR-014-009 | T014-026–028 |
| FR-014-010–011 | T014-002,029–033 |
| FR-014-012–013 | T014-034–039 |
| FR-014-014, NFR-014-001, SC-014-003 | T014-005,007,017,021,025,028,033,036–040 |
| FR-014-015, NFR-014-003, SC-014-006 | T014-004,008,014,018,021,029,034–036,039–040,042 |
| FR-014-016, NFR-014-004, SC-014-005 | T014-006,016,020,024,028,032,038–039,041 |
| FR-014-017, NFR-014-002, SC-014-004 | T014-006,008–009,024–025,029,031,033,037,040,043,045 |
| SC-014-002 | T014-017,021,025,028,032–033,038–041,043–044 |

## Incremental implementation strategy

The immediate MVP is US01 with realistic examples and observed results. The runtime MVP is the
accepted foundation plus US02, with all its faults and five-SDK parity; it does not advertise US03–07.
Continue story by story: failing contract/invariant tests → pure domain → transaction adapter →
public APIs/SDKs → failure/security/observability evidence → documented happy/failure demo. Existing
v1 workflows and their tests stay supported throughout. Roll back new publication/start first and
keep compatible readers for in-flight runs. No runtime implementation task is checked solely because its specification exists.
