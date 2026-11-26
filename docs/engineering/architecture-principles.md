# Architecture principles and how they are kept

Source of the principles: the constitution. This page says how a principle becomes a structure, a record and a check.

## Shape

A Rust **modular monolith**, a crate per bounded context, ports and adapters, dependencies pointing inwards: `domain ← application ← adapters ← apps`. A process is split out only after measurement. PostgreSQL is the authority; the transactional outbox and the inbox carry messages; the selected NATS/Redis transport is a hint (ADR-0069). Overview: `docs/architecture/overview.md`.

## Principles to checks (fitness functions)

An architectural rule that nothing checks is a wish. Each row is a rule and what fails when it is broken.

| Rule | Check that fails |
|---|---|
| Domain has no framework, SQL driver or broker dependency; layers point inwards; no cycles | `scripts/check-architecture.py` (CI `architecture`), tests in `scripts/tests/` |
| Every run is created with an input the gate checked (design 043): an `INSERT INTO runs` only in its three places; the legacy continue-as-new only in its forwarder; `test-support` only in dev-dependencies; `CheckedRunInput` built only by the gate; no `jsonschema` in the run-input validator; `tokio` in `sagat-application` only in `collection_source.rs` (ADR-0092); every creator a composition root builds given the gate | `scripts/check-architecture.py` (`run_creator_errors`, `test_support_errors`, `run_input_errors`, `application_runtime_errors`, `run_input_runtime_errors`, `gate_wiring_errors`), fixtures in `scripts/tests/test_check_architecture.py` |
| Every store statement is within the runtime role's privileges | `tests/integration/platform-foundation/runtime_privileges`; [supported static SQL forms](sql-privilege-audit.md) |
| Every service works as the least-privilege role | `SAGAT_TEST_AS_RUNTIME_ROLE=1` over the suites |
| Every variable-size column is classified | `storage-classification.json` + `integration_security_multitenancy` |
| Journal is the authority; projections equal a replay | `verify_run` / `assert_consistent` in the fault tests that compare a replay (54 of 92 in `fault_workflow_primitives` on 2026-11-01; the others assert persisted rows directly — inventory in `specs/014-workflow-primitives/validation-report.md`) |
| One transaction per transition (state, scheduling, timers, outbox) | fault tests with an injected failure per statement |
| Contracts only grow | `*.baseline.json` + compatibility tests; OpenAPI/AsyncAPI/proto/IR schema lint in CI `contracts` |
| A step an isolated runner executes is *managed* by its kind (SQL or lake), never by having a SQL operation: the worker paths ask `Node::managed_kind` (035, D035-134) | `crates/application/tests/managed_kind_sites.rs` (source-text check of `worker.rs` and `worker/`, not a call-graph proof; its own tests prove it sees the narrow question however it is laid out) |
| Six SDKs, one semantics | `tests/contract/sdk-conformance` (every scenario, six languages) |
| Tenant boundary: another tenant's resource is a missing one | `tests/contract/security-multitenancy/rest/battery.rs` over every operation |
| Observability vocabulary is closed and low-cardinality | telemetry tests (`observability.rs`, `approval_telemetry.rs`), metrics enums |
| Docs and generated references match the code | `contract_developer_experience`, `npm run check:api` |
| Every folder of `docs/` has an index that links every page in it, and `docs/README.md` links every folder | `scripts/check-docs-index.py` (CI `architecture`), tests in `scripts/tests/test_check_docs_index.py` |
| Agent guidance is bounded and consistent | `scripts/check-agent-config.py` |
| Incomplete capacity coordination stays dormant; new lease producers need inventory review | `scripts/check-capacity-activation.py` and negative fixtures (source-text check, not a call-graph proof); runtime lock/race tests remain required |
| Current-schema upgrade oracles follow the latest migration | `scripts/check-upgrade-boundary.py` and negative fixtures; runs before the released producer build; genuine released-source runtime checks remain required |
| A step-log charge never locks the tenant usage row or waits for a quota lease; only a refill, the top-up after a report, a release, an idle reclaim or a change of the limit locks the row; no transaction waits for a quota lease it does not own, and nothing waits after the usage row (a refill takes its own quota lease `NOWAIT`); a refill tries the row `SKIP LOCKED` and waits only while holding one of the tenant's *P* and one of the replica's *Q* wait slots (*Q* = max(1, ⌊N / 4⌋) of the replica's pool, *P* = ⌈*Q* / 2⌉), each lock acquisition at most 200 ms, and past it, or with no slot, raises `55P03`; the top-up never waits and takes no slot; the usage never exceeds the limit in force when the charge was admitted (ADR-0097, accepted 2026-11-20, OD-2026-11-20-15; decision 3b amended by OD-2026-11-20-18; the wait slots confirmed by OD-2026-11-21-9) | `integration_claim_transition_efficiency` `step_log_isolation` (design 041, task T041-5): the held-lock, deadlock and invariant tests (`leases::the_invariants_hold_under_random_interleavings`, `leases::a_refill_waits_for_a_held_usage_row_at_most_its_budget`, `leases::a_callers_lower_lock_timeout_is_kept`), the wait slots' (`wait_slots::*`) and the top-up's (`top_up::*`), the migration test of the 200 ms budget, and the source-text check `schema::only_the_top_up_the_refill_the_release_the_idle_reclaim_and_the_limit_change_lock_the_usage_row`; the carry's cases with a second tenant as the victim are task T041-36's |
| The step-log usage is read only as U = S + Σ used, through `sagat_quota_step_log_usage` (`STABLE`, so the quota view reads it in the snapshot of the grant, decision D041-141): no Rust source and no SQL outside the quota-lease functions reads `step_log_bytes` or `step_log_leased_bytes` (ADR-0097 decision 7) | `integration_claim_transition_efficiency` `step_log_isolation::schema::no_code_reads_the_folded_usage_or_the_leased_room_but_through_the_usage_function` (source-text check); `step_log_isolation::schema::the_quota_lease_functions_and_tables_have_exactly_their_privileges` (`provolatile = 's'`) |
| Step logs live beside the journal: no foreign key leads from `step_log_lines` or `step_log_streams` to `runs` or `steps` (ADR-0084 decision 1) | `integration_claim_transition_efficiency` `step_log_isolation::schema::no_foreign_key_leads_from_the_log_tables_to_runs_or_steps` (catalog check) |
| The hot path never reaches the step-log storage: the production code of `crates/application/src/worker.rs`, `worker/`, `execution*`, `crates/infra-postgres/src/execution.rs` and `execution/` never names `step_log_lines`, `step_log_streams` or `StepLogTx` (ADR-0084 decisions 1 and 2; task T041-36 restates it for `StepLogCarryTx`) | `integration_claim_transition_efficiency` `step_log_isolation::schema::the_hot_path_never_names_the_step_log_tables_or_their_transaction_port` (source-text check, not a call-graph proof) |
| The temporary Kafka dependency snapshot matches its registry source except for the reviewed zstd version | `scripts/check-vendor.py` and its negative controls (CI `architecture`) |
| Only succeeded originals enter a compensation plan; a terminal run has no open compensation (037) | `crates/domain/tests/execution_properties.rs` (`check`, `check_sagas`) |
| An unknown outcome is never retried, by any role (037) | `crates/domain/tests/sagas.rs`, `scoped_compensation.rs`; `contract_rest_sagas_compensations` (`rest/conformance.rs`, 409 `OUTCOME_UNRESOLVED`) |
| Stop and deadline keep a pending review (037) | `integration_sagas_compensations` `stopping::*` |
| Committed journals keep their meaning; v0.1/v0.2 journals replay (037, D1810) | `crates/domain/tests/legacy_replay.rs` (journals generated by the v0.2.0 domain) |
| The unknown-outcome code set is one list | `crates/domain/tests/outcome_unknown_parity.rs` (domain, SQL in `chain_progress.rs`, Studio `ops.ts`) |
| Review telemetry carries bounded codes only (037) | `crates/application/tests/telemetry_derive.rs` |
| Design docs keep their sections, ids, evidence, owner gates and a consistent approval record; the constitution's principle hashes agree with its Sync Impact Report, and a changed constitution raises its version (ADR-0077) | `scripts/check-design-docs.py` (CI `architecture`), tests in `scripts/tests/test_check_design_docs.py` |
| A review counts only if it ran with an accepted EGM plugin (ADR-0077) | `scripts/egm-tree-hash.py` against `docs/engineering/egm-accepted-hashes.json` (Rule 2 of `check-design-docs.py`) |
| The document leads and the code follows: a pull request that changes behaviour code changes a document, or each such commit says why not (`Docs: none, <reason>`) (constitution 4.7.0, Documentation first) | `scripts/check-docs-first.py` (CI `documentation`, every pull request), tests in `scripts/tests/test_check_docs_first.py`; it cannot see whether the right document changed, review does |
| Documents and code comments write words in full, not abbreviations such as "rule R8" or "architecture B1" (constitution 4.7.0, Words in full) | `scripts/check-plain-words.py` over the lines a pull request adds (CI `documentation`, every pull request), tests in `scripts/tests/test_check_plain_words.py`; the list is in `docs/governance/identifier-scheme.md` |
| The capability lists of the registry's three checks in SQL equal `Capability::ALL`, and each down migration restores the list in force before its up (feature 006, decisions D006-033 and D006-083) | `crates/infra-postgres/tests/capability_lists.rs` (every pull request that changes `infra-postgres` or `application`) and `scripts/check-architecture.py` |

When you add a rule worth keeping, add its row and its check in the same change.

## Architecture Decision Records

`docs/adr/`, one per decision, numbered, immutable once accepted (superseded, never edited). Write one when a decision is **hard to reverse, surprising without context, and the result of a real trade-off**. Otherwise it is a rule in a nested `AGENTS.md` or a `D…` entry.

An ADR here has: status, context (forces, not history), the decision stated as MUST / MUST NOT, at least two considered options with why they lost, consequences (good, bad, operational), **Enforced by** (the check above, or "review only"), and who decided and when. Decisions taken by an agent are reviewed by a person and say so; product and scope decisions are the owner's and also go to `docs/owner-decisions.md`. Template: `docs/adr/TEMPLATE.md`; index and rules: `docs/adr/README.md`.

## Quality attributes are scenarios

State each as stimulus, environment, response and measure (`docs/architecture/observability-slo.md`, `capacity-and-topology.md`, the plan's performance section). Targets are not promises: a benchmark report names the hardware, the load and the failures (`tests/performance/reports/`). A ceiling is accepted only when a measured run supports it; when it does not, the limit is published lower and the gap stays open.

## Evolving safely

- Additive migrations and contracts; readers tolerate what they do not know; a rolled-back reader keeps working or refuses with a typed reason.
- Published workflow versions are immutable; a run references exactly one.
- Small batches: a pull request closes one coherent increment with its evidence; long-lived branches merge `develop` often.
- Degrade instead of fail: the platform works by polling when the broker is down.
- Threat model per trust boundary (`docs/architecture/security-and-sandbox.md`); untrusted code only ever runs isolated.

## Records and where they live

| Record | Place |
|---|---|
| Principle | constitution |
| Structural decision | ADR |
| Feature decision, gap, question | `specs/NNN-slug/research.md` (`D`, `G`, `O`) |
| Product/scope/release decision | `docs/owner-decisions.md` |
| Operational procedure | `docs/runbooks/` |
| Evidence of a measured claim | `validation-report.md`, `tests/performance/reports/` |

## Recovery checks (feature 037)

The rows marked 037 above enforce delivery 1 (Constitution 2.0.0, principle III). Qualification,
finality and authority exceptions remain open under US04 and have no check yet: none is enforced.
