# Open-task review dossier

## Review purpose

Review every unchecked task entry in the current checkout for scope clarity, consistency with its specification, remaining evidence, dependencies, and implementation readiness. Produce findings and required follow-up artifacts. Do not implement features, change task checkboxes, publish packages, accept performance limits, or simulate human evaluation.

## Snapshot and scope

- Date: 2026-10-30.
- Checkout: `develop` at `888074e6237b2a1ce9c85f83ec5703b081c65dd6`.
- Inventory: 12 unchecked entries in 3 task files; 18 canonical task inventories inspected.
- Exact source hashes, line locations and related worktrees: [inventory.json](./inventory.json).
- The same human-acceptance identifiers occur in two entries in `tasks/todo.md`; retain both source locations until review determines their relationship.
- Existing README bootstrap edits are documentation in progress, not new runtime or acceptance evidence.
- Related worktrees may contain work not integrated here. A task unchecked here is not proof that no implementation exists elsewhere; distinguish integrated evidence, branch claims and missing evidence.

## System and review baseline

- [.specify/memory/constitution.md](../../../.specify/memory/constitution.md)
- [AGENTS.md](../../../AGENTS.md)
- [specs/AGENTS.md](../../../specs/AGENTS.md)
- [SPECIFICATION-INDEX.md](../../../SPECIFICATION-INDEX.md)
- [docs/traceability.md](../../../docs/traceability.md)
- [docs/owner-decisions.md](../../../docs/owner-decisions.md)
- [docs/test-strategy.md](../../../docs/test-strategy.md)
- [docs/sdk-compatibility.md](../../../docs/sdk-compatibility.md)
- [docs/runbooks/sdk-packaging.md](../../../docs/runbooks/sdk-packaging.md)
- [specs/014-workflow-primitives/spec.md](../../../specs/014-workflow-primitives/spec.md)
- [specs/014-workflow-primitives/plan.md](../../../specs/014-workflow-primitives/plan.md)
- [specs/014-workflow-primitives/tasks.md](../../../specs/014-workflow-primitives/tasks.md)
- [specs/014-workflow-primitives/research.md](../../../specs/014-workflow-primitives/research.md)
- [specs/014-workflow-primitives/validation-report.md](../../../specs/014-workflow-primitives/validation-report.md)
- [specs/033-redis-streams/spec.md](../../../specs/033-redis-streams/spec.md)
- [specs/033-redis-streams/plan.md](../../../specs/033-redis-streams/plan.md)
- [specs/033-redis-streams/tasks.md](../../../specs/033-redis-streams/tasks.md)
- [specs/033-redis-streams/research.md](../../../specs/033-redis-streams/research.md)
- [specs/033-redis-streams/validation-report.md](../../../specs/033-redis-streams/validation-report.md)
- [specs/006-sdk-multilanguage/spec.md](../../../specs/006-sdk-multilanguage/spec.md)
- [specs/006-sdk-multilanguage/plan.md](../../../specs/006-sdk-multilanguage/plan.md)
- [specs/006-sdk-multilanguage/tasks.md](../../../specs/006-sdk-multilanguage/tasks.md)
- [specs/006-sdk-multilanguage/research.md](../../../specs/006-sdk-multilanguage/research.md)
- [specs/008-observability-operations/spec.md](../../../specs/008-observability-operations/spec.md)
- [specs/008-observability-operations/plan.md](../../../specs/008-observability-operations/plan.md)
- [specs/008-observability-operations/tasks.md](../../../specs/008-observability-operations/tasks.md)
- [specs/008-observability-operations/research.md](../../../specs/008-observability-operations/research.md)
- [specs/012-developer-experience/spec.md](../../../specs/012-developer-experience/spec.md)
- [specs/012-developer-experience/plan.md](../../../specs/012-developer-experience/plan.md)
- [specs/012-developer-experience/tasks.md](../../../specs/012-developer-experience/tasks.md)
- [specs/012-developer-experience/research.md](../../../specs/012-developer-experience/research.md)
- [docs/validation/human-acceptance-results.template.md](../../../docs/validation/human-acceptance-results.template.md)
- [docs/validation/human-acceptance.md](../../../docs/validation/human-acceptance.md)

Reviewers must read their assigned EGM persona/standards, this dossier, the original task files, the applicable feature specification/plan, and the source/evidence references relevant to their findings. Existing test counts are historical claims until independently verified; a documentation review does not rerun the runtime gates. Missing or inaccessible evidence must be reported rather than assumed successful.

## Open-work overview

The open entries cover collection execution/admission and its integrated fault/performance/release acceptance, Redis integrated validation, public SDK registry publication, and genuine human usability evaluation. Several entries already include partial evidence, so the review must identify the remaining acceptance boundary rather than describe all work as absent.

## Detailed Implementation Plan

This section preserves the open task wording and adjacent evidence from the source inventories. It is the implementation-readiness target, not a newly approved implementation plan. Review-entry labels identify source locations only and do not replace or renumber feature identifiers.

### OT-001 — specs/014-workflow-primitives/tasks.md:63

Source: [specs/014-workflow-primitives/tasks.md:63](../../../specs/014-workflow-primitives/tasks.md#L63).

- [ ] T014-014 [P] [US02] Add failing collection/model/tenant contention cases in `crates/domain/tests/primitive_collections.rs` and `tests/integration/workflow-primitives/collections.rs`; assert “maxItems and maxIterations are integers in 1..10000”, “maxConcurrency is 1..256”, snapshot immutability and shared run/tenant budgets, descendant admission with low quotas and derived-instance reservations that survive policy reduction. Refs: FR-014-003–005,015, NFR-014-003.
  _Evidence: Clause→test table in `research.md` ("Evidence by clause"): `ceilings.rs` (maxItems/maxIterations 1..10000, maxConcurrency 1..256, 6 tests), `primitive_collections.rs` (3), `collection_budgets` (retry charged once, a reduced budget still pays for compensation, capacity of one runs every iteration), `a_collection_entered_once_keeps_its_membership_against_any_later_entry` (snapshot immutability), the shared run/tenant budget and descendant-admission cases in `collections_http`/`limits`. No nested-scope test: nested scopes are closed._

### OT-002 — specs/014-workflow-primitives/tasks.md:65

Source: [specs/014-workflow-primitives/tasks.md:65](../../../specs/014-workflow-primitives/tasks.md#L65).

- [ ] T014-015 [US02] Implement typed collection entry/admission/ordered aggregation/fail-fast drain in `crates/domain/src/execution/{instance.rs,plan.rs,state.rs,event.rs}` and definition rules in `crates/domain/src/workflow.rs`; charge each admitted logical instance once and release slots only after terminal/reconciled drain. Refs: FR-014-004–005,014.
  _Evidence: Domain `crates/domain/tests/collection_reducer.rs`, `collection_restore_state.rs`, `scoped_reducer.rs`; each admitted logical instance is charged once and slots are released only after the drain (`collection_budgets`, `scope_compensation`); ordered aggregation and fail-fast drain in `collections_http` and `fault/…/collections.rs`._

### OT-003 — specs/014-workflow-primitives/tasks.md:67

Source: [specs/014-workflow-primitives/tasks.md:67](../../../specs/014-workflow-primitives/tasks.md#L67).

- [ ] T014-016 [US02] Wire captured arrays/blob refs and transactional collection progress in `crates/application/src/primitives.rs` and `crates/infra-postgres/src/primitives.rs`; add equivalent forEach/map builders to `sdks/{typescript,go,python,dotnet,java}/` and reviewed WP-03 outputs in `tests/contract/sdk-conformance/expected/`. Refs: FR-014-004,016–017.
  _Evidence: Blob item sources: `collection_sources` (10 tests; 8 red first) — a run-input blob array is the membership, large items reach their iteration as references, the membership is fixed at entry and the blob behind it is protected (`iteration_scopes.source_blob_id` RESTRICT FK, migration `20261121000000`), foreign/corrupt/missing blob → `BLOB_UNAVAILABLE`/`BLOB_CORRUPT`, store outage waits; fault `collection_sources` (5). Builders in the five SDKs and scenarios `collection-builders`, `collection-runtime`, `collection-ordering`, `collection-blob-source`, `collection-limits` pass in all five without bless. G420 now has public pipeline evidence: ordinary root source prefixes supply inline arrays/committed output blobs; `collection_pipeline` proves success, exact report replay, source failure and uncommitted-blob refusal. Minor 1.4 produces root work but cannot lease admitted body work. The five-SDK `collection-step-source` fixture preserves authored limits, equal root/body node ids with distinct paths and local-only contexts. Broader source effect/error combinations and nested/broader bodies remain open._

### OT-004 — specs/014-workflow-primitives/tasks.md:69

Source: [specs/014-workflow-primitives/tasks.md:69](../../../specs/014-workflow-primitives/tasks.md#L69).

- [ ] T014-017 [US02] Prove crash/duplicate/stale-fence recovery, lost broker reconciliation, cancellation and unknown/failed compensation in `tests/fault/workflow-primitives/collections.rs`; verify FR-003-002 compensation order (reverse topological within an iteration, no order between collection iterations) and all-five WP-03/04 conformance. Refs: FR-014-005,014, NFR-014-001, SC-014-002–004.
  _Evidence: `fault/…/collections.rs` (crash, duplicate, stale fence, lost broker hints, cancellation), `fault/…/scope_compensation.rs` (unknown/failed compensation, reverse topological order inside an iteration, no order between iterations), `collection-compensation` and the WP-03/04 collection scenarios in all five SDKs. Replica takeover test made deterministic (G407)._
  _2026-10-27 compatibility increment: actual immutable released-v0.2 public producer → normal current migration/completion → released REST/native reader passes1 coordinator in8.42s; nested old phases and exact source/executable provenance are retained in [released-v02 evidence](../../../specs/014-workflow-primitives/evidence/released-v02-20261027/README.md). This covers old READY/unleased activity and root wait only; old in-flight reports, scoped-dispatcher cutover and the remaining complete runtime/fault acceptance stay open._

  _2026-10-27 in-flight increment: actual released worker retains its original live scoped FORWARD request/capability across normal upgrade, completes and replays against current services; coordinator1/0/0 in6.25s, old phase2.67s. Exact receipt/accounting/capability, immutable replay and2current scheduler ticks are retained in [in-flight evidence](../../../specs/014-workflow-primitives/evidence/released-v02-inflight-20261027/README.md). Applied/reconciliation, old-dispatcher cutover, nesting/capacity and complete fault/release acceptance remain open._

  _2026-10-27 Applied increment: actual released uncertain forward/lookup → current Applied/exact replay → actual released terminal REST/native/prefix reader passes1 coordinator in6.37s, with exact capabilities, events, payload, receipt and accounting checks. See [Applied evidence](../../../specs/014-workflow-primitives/evidence/released-v02-applied-20261027/README.md). Quota/drain/original-forward compensation frontier, dispatcher activation and complete runtime/fault/release criteria remain open._

### OT-005 — specs/014-workflow-primitives/tasks.md:189

Source: [specs/014-workflow-primitives/tasks.md:189](../../../specs/014-workflow-primitives/tasks.md#L189).

- [ ] T014-040 Execute the complete barrier/race/duplicate/lost-broker/failed-compensation/restart and tenant/secret matrix in `tests/fault/workflow-primitives/` and `tests/integration/workflow-primitives/`; retain seeds, commands, failed/skipped results and independently inspected persistence in `specs/014-workflow-primitives/validation-report.md`. Refs: FR-014-014–015, NFR-014-001–002, SC-014-002–004.
  _Evidence: The fault matrix by class (crash, duplicate, race, lost broker, failed outbox/audit, stale fence, failed compensation, restart, tenant, secret) with the tests of each is in `validation-report.md`, "Release-gate checkpoint"; `fault_workflow_primitives` 87/87 locally and in CI run 39057216527 (head 7d7868ff), also as the runtime role; every fault test ends with `verify_run`/`assert_consistent`. Failures seen on the way and what they were are listed there (G407, G467, the scale-merge fixes)._

### OT-006 — specs/014-workflow-primitives/tasks.md:194

Source: [specs/014-workflow-primitives/tasks.md:194](../../../specs/014-workflow-primitives/tasks.md#L194).

- [ ] T014-042 Measure plan workloads with `tests/performance/workflow-primitives/` and publish hardware/configuration/raw summaries/failures in `tests/performance/reports/workflow-primitives.md`; prove cross-run tenant concurrency bounds and zero dedicated workers for waits before accepting production ceilings. Refs: FR-014-015, NFR-014-003, SC-014-006.
  _Evidence: Harness `tests/performance/workflow-primitives/` and report `tests/performance/reports/workflow-primitives.md` (hardware, commands, raw numbers, failures). Proven as assertions: waits cost no worker, thread or connection (10,000 approvals + 1,000 timers pending: 4 threads, 8 connections, 0 sessions, no lease), and the tenant slot bound was never exceeded in 279 rounds. Production ceilings are NOT accepted yet: a 10,000-item scope costs quadratic work (G461, 1 h 54 min) and idle ticks grow with pending waits (G468); fix in progress on `feature/014-scale`, ceilings provisional (O460, O463)._

### OT-007 — specs/014-workflow-primitives/tasks.md:196

Source: [specs/014-workflow-primitives/tasks.md:196](../../../specs/014-workflow-primitives/tasks.md#L196).

- [ ] T014-043 Run formatting, Clippy, locked workspace, architecture, canonical contract compilation, Compose smoke and dependency advisory gates; update primitive operator guidance in `docs/runbooks/workflow-primitives.md`, schema/API reference and `specs/014-workflow-primitives/quickstart.md` with happy/failure demos and compatible-reader rollback. Refs: FR-014-017, NFR-014-001–004, SC-014-002–006.
  _Evidence: CI run 39057216527 on head 7d7868ff, all 12 checks green: `rust (fmt, clippy, tests)` (`cargo test --workspace --locked --no-fail-fast`), `contracts`, `architecture rules`, `dependency advisories (cargo audit)`, `local environment (docker compose smoke test)`, `sdks` (unit tests, 51-scenario conformance in five languages, user stories, faults), `studio`, `native Go and Java` (both architectures), `observability files`, `package and clean-install smoke tests`, `scale and recovery`. Gates the first CI round found and that were fixed: ajv strict mode on `x-maxUtf8Bytes`, stale Studio API types, mypy/ruff/`dotnet format`, protocol 1.5 expectations, CLI help for nine codes, cross-tenant battery bodies, rollback guard. Operator guidance: `docs/runbooks/workflow-primitives.md`, quickstart demos (loops, approvals with state and chains), `docs/quickstarts/errors.md`. Compatible-reader rollback: `compatibility.rs` and the rollback test._

### OT-008 — specs/014-workflow-primitives/tasks.md:198

Source: [specs/014-workflow-primitives/tasks.md:198](../../../specs/014-workflow-primitives/tasks.md#L198).

- [ ] T014-044 Update `docs/traceability.md`, dependent feature docs and `specs/014-workflow-primitives/{tasks.md,validation-report.md}` with requirement→test→contract→metric/log→demo evidence; enable only fully proven primitives and leave missing tests unchecked. Refs: FR-014-003–017, SC-014-002–006.
  _Evidence: `docs/traceability.md` section 014 rewritten requirement by requirement (2026-10-17) with tests, contracts, scenarios and the open items; `validation-report.md` has the release-gate checkpoint; primitives are enabled only in the shapes proven, and the missing ones stay listed as gaps (T014-004 fixtures closed 2026-10-18; nested runtime and production ceilings remain unaccepted, G461/G520)._

### OT-009 — specs/033-redis-streams/tasks.md:15

Source: [specs/033-redis-streams/tasks.md:15](../../../specs/033-redis-streams/tasks.md#L15).

- [ ] T033-007 Run appropriate format/lint/static/contract/integration gates; update validation report,
  traceability, roadmap and release0.3 dependency evidence. Tick only obtained acceptance.


Evidence for T033-001..006: [validation report](../../../specs/033-redis-streams/validation-report.md), actual gates dated2026-10-27.
T033-002:27 engine configuration cases plus28 Redis cases and111 existing event regressions.
T033-003: public API/schema/tenant/runtime-role lifecycle and guarded migration cases.
T033-004: real TLS/ACL/rotation/DB/pinned-DNS/secret/boot cases in tls.rs/bootstrap.rs.
T033-005: actual lost wire ACK, crash/stale publisher, outage/DEAD/redrive and111 legacy cases.
T033-006: actual Compose Healthy/AOF/loopback proof and teardown;25 Helm contracts; Studio type drift,
typecheck/lint and canonical REST/AsyncAPI contracts.
T033-007: actual old callback writer/reader, login/session revocation, process join before Redis
activation and preserved old rows now pass; six source-bound released profiles, 139 messaging/
Redis regressions and five common scenarios in six SDKs pass on the combined tree. Evidence:
[released-callback-cutover.md](../../../specs/033-redis-streams/released-callback-cutover.md). The final release-tree gates remain
open; no acceptance is inferred from the default-off flag.

### OT-010 — tasks/todo.md:28

Source: [tasks/todo.md:28](../../todo.md#L28).

- [ ] SC-008-001 / SC-012-001: agree the numeric UX target and record real human evaluation; never substitute an automated timing claim.

### OT-011 — tasks/todo.md:71

Source: [tasks/todo.md:71](../../todo.md#L71).

- [ ] FR-006-008: publish verified SDK artifacts under confirmed registry identities, record immutable versions and consumer evidence (awaiting registry identity information).

### OT-012 — tasks/todo.md:72

Source: [tasks/todo.md:72](../../todo.md#L72).

- [ ] SC-008-001 / SC-012-001: record genuine human usability evaluation under the existing protocol; no simulated human acceptance.

## Review deliverables

The selected EGM preset controls the stage reports. A full review produces initial zoom-out, comprehension, critic, intermediate zoom-out, implementation-readiness and consolidated reports. The consolidated report must map every inventory entry to reviewed findings or an explicit no-finding result, distinguish duplicated requirements, list missing evidence and owner dependencies, and map raw findings to prioritized follow-up actions. Suggested source changes require a separate authorized resolution step.
