# Tasks — 029 custom roles, groups, folders and per-operation access lists

Status: Proposed — not started. Each task names the evidence that closes it. Tests come first and must
fail first. Rust suites use real PostgreSQL; store and HTTP suites also run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.

Format: `- [ ] T029-NNN [P?] [USn?] Description (paths). Refs. Evidence.` `[P]`: different files, no open dependency.

## Phase 1 — Setup and prerequisite release

- [ ] T029-001 Owner decision on ADR-0064 and O1440–O1442, recorded in `docs/owner-decisions.md`, `docs/adr/README.md` and the ADR; "Refined by ADR-0064" notes for ADR-0018, 0028, 0031, 0035, 0036 once accepted; dependency notes in 013, 018, 019, 020, 021, 022, 024, 025, 026, 027, 028, 030, 031, 032 (by their owners): permissions and machine roles to classify, and the evaluator adoption of 018 schedules, 020 list queries and saved views, 021 resources (folder placement), 025 MCP listing and calls, 026 links. Refs: all. Evidence: merged change.
- [ ] T029-002 [P] Targets `{integration,contract_rest,fault,e2e,perf}_custom_roles_folders` in `tests/Cargo.toml`, `main.rs` in `tests/{integration,contract,fault,e2e}/custom-roles-folders/`. Evidence: `--list` shows each suite.
- [ ] T029-003 Prerequisite release vehicle (ships before 029): deploy feature 018's instance-capability registry (T018-080: `serving_instances`, `platform_requirements`, `platform_requirement_epoch`, registration before serving, heartbeat, per-admission and per-stream re-check, `BINARY_CAPABILITY_MISSING`) fleet-wide; activation of `access-narrowing-v1` (kind `CONTROL_PLANE`) is the operator's audited 018 command, not a tenant transaction; 029 adds the check, used by T029-031/T029-046, that refuses narrowing writes (`ACCESS_CAPABILITY_DISABLED`) until the requirement is recorded, and the removal precondition (no restricted folder or list remains). Depends on T018-080. Refs: FR-029-037. Evidence: integration test with a live older replica (narrowing refused before activation; activation refused while the older replica is live; `503` after activation, including after a pause beyond the liveness window) (SC-029-017).

## Phase 2 — Foundational (blocks every story)

- [ ] T029-004 [P] Fixtures `tests/contract/custom-roles-folders/fixtures/{evaluation-vectors,permission-catalog}.json` from `contracts/` (V01–V36). Refs: FR-029-013, FR-029-015, FR-029-019, FR-029-020. Evidence: loaded by T029-005.
- [ ] T029-005 `crates/domain/src/access.rs`: classification (`match` with explicit arms and a compile-time default arm of reserved; a contract test lists unclassified permissions), `FolderLevel`, `AccessFacts`, `Target`, `Decision` with sources, `evaluate` (decision order: permission before lookup, then visibility), `visible_folder`, `visible_folders(kind)`, `can_grant`, `can_place`, `can_create_in`, `can_widen_list`, `can_add_member`; classification arms with a compile-time default of reserved; no SQLx/Axum/`serde_json`. Refs: FR-029-002, FR-029-003, FR-029-006, FR-029-008, FR-029-012, FR-029-013, FR-029-015, FR-029-017, FR-029-019, FR-029-020. Evidence: all vectors (V01–V36); property tests (permutation, monotonicity, reserved exclusion, built-in floor, developer bound, kind check, listing equivalence, no oracle) over 10,000 cases (SC-029-002).
- [ ] T029-006 Five permissions in `crates/domain/src/identity.rs` (`ADMIN_PERMISSIONS` +5; `Role` doc comment); `Principal` carries `tenantWide`. Tests in `crates/domain/tests/identity.rs` assert set properties, not counts (admin holds every assignable classified permission; the five go to admin only; developer holds every level permission; human roles are exactly admin, developer, operator, viewer; any other role is machine). Refs: FR-029-001, FR-029-002. Evidence: tests still pass after another feature adds a role or grant (simulated in the test).
- [ ] T029-007 Access migration per `data-model.md`: `custom_roles` (names unique per owner), `custom_role_revisions` ("cardinality from 1 to the number of assignable permissions", immutable), `role_assignments`, `access_groups`, `access_group_members`, `folders`, `folder_entries` ("≤ 256 per folder"), `item_placements` (one per item, `ON DELETE CASCADE` to items, `RESTRICT` to folders, no private 017 definitions), `operation_lists` ("at most 64 per workflow"), `operation_list_principals` ("≤ 128 per list"); `UNIQUE (tenant_id, namespace_id, id)` on `scripts`; human-role and namespace-role-scope triggers; role-change `SECURITY DEFINER` trigger; quota resource names; `sagat_app` grants; `storage-classification.json` ("description ≤ 512 UTF-8 bytes", "display_name ≤ 128 bytes"). Down refuses while rows exist. Refs: FR-029-008, FR-029-010, FR-029-011, FR-029-024, FR-029-030, FR-029-034, FR-029-035. Evidence: `integration_platform_foundation` (migrations, `runtime_privileges`, `least_privilege`); classification test.
- [ ] T029-008 [P] Audit actions and resource types (`crates/domain/src/audit.rs`); reasons and remediation rows (`crates/api-http/src/error.rs`). Refs: FR-029-027, FR-029-028. Evidence: closed-mapping tests like `routes/reveal_grants.rs`.
- [ ] T029-009 [P] Metric families incl. `access_fence_wait_seconds` and span attributes as closed enums (`crates/telemetry/src/metrics.rs`). Refs: FR-029-029. Evidence: label-set tests.
- [ ] T029-010 [P] Quota resources `custom_roles`, `access_groups` (tenant formula) and `folders` (namespace formula) in `crates/domain/src/quota.rs`; namespace-role bound "20 (hard 100)". Refs: FR-029-030. Evidence: quota tests.
- [ ] T029-011 `crates/infra-postgres/src/access.rs`: one-statement fact query, fences with writer `lock_timeout`, `KEY SHARE NOWAIT` helper with ≤ 3 retries, `effective_permission_in_tx`, one statement that reads item and placement for both missing and hidden targets, listing queries that take the domain's `visible_folders(kind)` set as a parameter (no SQL re-derivation); `begin_safely`. Refs: FR-029-020, FR-029-021. Evidence: integration as `sagat_app`; `runtime_privileges`.
- [ ] T029-012 Lock-order inventory (every caller of `effective_permission_in_tx`, item creation in a folder, publication) against ADR-0050 decision 7 (029 capability check as a read in slot 1; memberships → 029 access fences → namespace row in slot 4a, after slots 2, 3 and 3R) and the 014 and 017 orders, including folder and list creation's `FOR SHARE` read of 018's registry epoch against the operator's activation and removal; barrier tests in `tests/integration/custom-roles-folders/lock_order.rs`: move vs publish, move vs connection admission, folder delete vs create-in-folder, long reader vs queued writer (`ACCESS_WRITE_CONTENDED`). Refs: FR-029-021, NFR-029-DUR. Evidence: suite passes; no deadlock in `pg_stat_database`.
- [ ] T029-013 `crates/application/src/access.rs`: `AccessStore` port and `Authorizer` (`require`, `require_item`, `visible_folders`, `explain`). Refs: FR-029-001, FR-029-020. Evidence: unit tests over a fake on domain types.
- [ ] T029-014 Typed access extractors and a generated route registry (`crates/api-http/src/routes/mod.rs`); `accessControl` per kind in `GET /version`. Refs: FR-029-022. Evidence: registry equals `x-access` for every handler.
- [ ] T029-015 `x-access` (with `discloses`) on every operation of `specs/001-platform-foundation/contracts/openapi.yaml`; `tests/contract/custom-roles-folders/coverage.rs`. Refs: FR-029-022. Evidence: fails when one declaration or `discloses` is removed (SC-029-005).
- [ ] T029-016 Rule in `scripts/check-architecture.py` failing on `Role::grants`, `Principal::authorize` or `== Role::` outside `domain::access` and a machine-route allowlist; row in `docs/engineering/architecture-principles.md`; tests in `scripts/tests/`. Refs: FR-029-020, FR-029-021. Evidence: rule fails on a planted violation.
- [ ] T029-017 After T029-012, switch every store recheck of the plan's inventory (`crates/infra-postgres/src/{declared_operations,approval_progress,event_payloads,payload_disclosure,managed_secrets,reveal_grants,database,database_results,secrets}.rs`, `database/admission.rs`) to `effective_permission_in_tx`. Refs: FR-029-021. Evidence: `integration_{workflow_primitives,payload_classification,database_connections,security_multitenancy}` as `sagat_app`.

## Phase 3 — US01 Define a custom role and assign it (P1) — MVP

- [ ] T029-018 [P] [US1] `tests/integration/custom-roles-folders/roles.rs` for US01 scenarios 1–6. Refs: FR-029-003–005. Evidence: fails first; part of SC-029-001.
- [ ] T029-019 [US1] Role use cases and store: revision CAS, immutable revisions, `ROLE_IN_USE`, `RESERVED_PERMISSION`, `UNKNOWN_PERMISSION`, `PERMISSION_SCOPE_MISMATCH`, stored scope, quota, "at most 1,000 revisions per role". Refs: FR-029-003, FR-029-004, FR-029-030. Evidence: T029-018.
- [ ] T029-020 [US1] Assignment use cases: idempotent per (role, principal, scope); human-only with `PRINCIPAL_NOT_ELIGIBLE` for callers without `access:read`; namespace roles only in their namespace; non-escalation under the fence; "Direct assignments per subject 32 (hard 64)". Refs: FR-029-005, FR-029-006, FR-029-007. Evidence: T029-018.
- [ ] T029-021 [US1] REST routes `crates/api-http/src/routes/access.rs`, `dto_access.rs` (catalog, roles, revisions, assignments) and `tests/contract/custom-roles-folders/rest.rs`. Refs: FR-029-004, FR-029-005. Evidence: contract tests.
- [ ] T029-022 [US1] Migrate role-only decisions for definitions, publishing, versioning (`crates/application/src/{definitions,publishing,versioning}.rs`). Refs: FR-029-001, FR-029-020. Evidence: built-in matrix unchanged without custom facts; custom grants honoured.
- [ ] T029-023 [US1] Migrate runs, operations, chains, declared operations, approvals, payloads, rate admission (`crates/application/src/{runs,operations,chain_progress,primitives,payloads,payload_disclosure,event_payloads,api_admission}.rs`, `runs/ad_hoc.rs`, `crates/domain/src/execution/approval.rs`). Refs: FR-029-001, FR-029-017, FR-029-020. Evidence: as T029-022; rate charge of hidden = missing.
- [ ] T029-024 [US1] Migrate scripts, database, secrets (admin bypass of lists stays tied to built-in `admin` in `crates/domain/src/secrets.rs`; `secret:manage` and `worker:bind` stay reserved), bindings, quotas, events, connectors, outbound, egress, identity and the route checks in `crates/api-http/src/routes/{outbound,runner}.rs`. Refs: FR-029-001, FR-029-003, FR-029-020. Evidence: as T029-022; T029-016 rule passes.
- [ ] T029-025 [P] [US1] `sagat access role|assign` in `apps/cli/src/access.rs` and `args.rs`. Refs: FR-029-032. Evidence: CLI tests.
- [ ] T029-026 [P] [US1] Every new operation in `tests/contract/security-multitenancy/rest/battery.rs`. Refs: FR-029-035. Evidence: battery passes; an entry missing fails.

## Phase 4 — US02 Local groups (P1)

- [ ] T029-027 [P] [US2] `tests/integration/custom-roles-folders/groups.rs` for US02 scenarios 1–5. Evidence: fails first.
- [ ] T029-028 [US2] Group and member use cases, store, REST, CLI: flat, human-only, local only, `GROUP_IN_USE`, `canAddMember` with widening audits for each list and restricted entry naming the group, "Members per group 1,000 (hard 5,000)", "Groups per subject 32 (hard 64)", "entries and list principals naming one group 128 (hard 256)". Refs: FR-029-006, FR-029-008, FR-029-030. Evidence: T029-027 (US02 scenario 6).
- [ ] T029-029 [US2] Membership deletion cascade and role-change trigger tests (operator actions as owner). Refs: FR-029-008, FR-029-024. Evidence: one `access.revoked_on_role_change` audit per affected subject.

## Phase 5 — US03 Folders, then restriction for workflows and runs (P1)

- [ ] T029-030 [P] [US3] `tests/integration/custom-roles-folders/folders.rs` for US03 scenarios 1–8. Evidence: fails first.
- [ ] T029-031 [US3] Folder use cases, store, REST, CLI: create/rename/delete (`FOLDER_NOT_EMPTY` over live items), `visibleFolder`, entries with non-escalation, owner entries need folder administration, mode with `expectedRevision`. Refs: FR-029-010, FR-029-014. Evidence: T029-030.
- [ ] T029-032 [US3] Placement: create in a folder, `moveWorkflow` with `canPlace`, manifest placement through the same use case, runs follow the workflow's folder. Refs: FR-029-011, FR-029-012, FR-029-018, FR-029-025. Evidence: integration tests.
- [ ] T029-033 [US3] Filtered listings and disclosing operations (workflows, runs, chains, approvals, alerts, event inbox and dead letters, publication requests), all using the domain's `visible_folders(kind)`; hidden-item probe audits off the response path. Refs: FR-029-017, FR-029-020. Evidence: listing tests; property test listing ≡ `visible(i)`; query plans at 1,000 folders.
- [ ] T029-034 [US3] Name-keyed writes and replays resolve visibility first: workflow create, promotion target, manifest publish, run-start key replay. Refs: FR-029-017, FR-029-025. Evidence: hidden rows byte-identical before and after.
- [ ] T029-035 [US3] Preview: counts (subjects, groups, approvals, runs, dependent parents) of items visible to the previewer only, one opaque total for the rest to `folder:manage`/`access:read` holders, principal sample only for `access:read`. Refs: FR-029-016. Evidence: preview/apply race (`REVISION_CONFLICT`); a hidden item never identified by a preview.
- [ ] T029-036 [US3] Opacity battery `tests/contract/custom-roles-folders/opacity.rs` for workflow and run kinds and folders, as every built-in human role (viewer, developer, operator, admin) and a level-only member, incl. rate effect and the `403`-before-lookup cases (V31–V35). Refs: FR-029-010, FR-029-017, FR-029-022, NFR-029-SEC. Evidence: SC-029-004.
- [ ] T029-037 [US3] Canary scan `tests/contract/custom-roles-folders/canary_scan.rs` over every implemented operation except audit reads (G1451, asserted as an explicit exclusion list). Refs: FR-029-022. Evidence: SC-029-004, SC-029-005.
- [ ] T029-038 [US3] Stream re-evaluation every ≤ 30 s and on resume (`crates/api-http/src/streams.rs`). Refs: FR-029-021. Evidence: SC-029-008.
- [ ] T029-039 [US3] Enable `restrictedFolders` for workflow and run kinds only when T029-015, T029-036, T029-037 and T029-003 pass; restriction is refused until the operator has activated `access-narrowing-v1` in 018's registry. Refs: FR-029-022, FR-029-037. Evidence: gate test.
- [ ] T029-040 [P] [US3] Manifest `folder` in `specs/011-gitops-versioning/contracts/manifest.schema.json`, `apps/cli/src/manifest.rs`; promotion `destinationFolder` rules. Refs: FR-029-025. Evidence: `contract_manifest_gitops_versioning`, `integration_gitops_versioning` (SC-029-015).
- [ ] T029-041 [US3] Publication reference checks in `crates/application/src/{definitions,publishing}.rs`: `run:start` on children, `script:read`, `database:use`; triggers with a level-only publish right need `event:manage`. `Authorizer::recheck_for_subject` for actions on behalf of a recorded subject, handed to 018 (schedule actions at fire time) and 026 (link submissions). Refs: FR-029-023. Evidence: integration tests; US03 scenario 10 with 018/026 fixtures when those features are present.

## Phase 6 — US04 Delegation without escalation (P2)

- [ ] T029-042 [P] [US4] Escalation matrix `tests/integration/custom-roles-folders/escalation.rs` (roles, assignments, entries, creates, moves, manifest placement, list widening, group member addition). Refs: FR-029-006, FR-029-012, FR-029-019, NFR-029-SEC. Evidence: SC-029-003.
- [ ] T029-043 [US4] Namespace roles and delegated `role:manage`, `role:assign`, `folder:manage`; barrier test for US04-6. Refs: FR-029-004–006, FR-029-014. Evidence: T029-042; barrier.
- [ ] T029-044 [US4] Listing by granter; assignments survive a granter's loss of authority. Refs: FR-029-007. Evidence: integration test.

## Phase 7 — US05 Operation lists (P2)

- [ ] T029-045 [P] [US5] `tests/integration/custom-roles-folders/operation_lists.rs` (run- and chain-addressed, replay after removal, ahead of declaration, empty list, widening refused). Evidence: fails first.
- [ ] T029-046 [US5] List use cases, store, REST, CLI with the narrowing and widening rules; "Principals per operation list 32 (hard 128)". Refs: FR-029-019. Evidence: T029-045.
- [ ] T029-047 [US5] Enforce lists in `crates/infra-postgres/src/declared_operations.rs` after current authorization and before receipt replay. Refs: FR-029-019. Evidence: T029-045; `integration_workflow_primitives`.
- [ ] T029-048 [US5] Enable `operationLists` under the T029-039 gate. Refs: FR-029-022, FR-029-037. Evidence: gate test.

## Phase 8 — US06 Explanation and Studio (P2)

- [ ] T029-049 [P] [US6] `tests/integration/custom-roles-folders/explain.rs`: explanation equals the evaluator on every vector; hidden targets and folders `404`. Evidence: fails first.
- [ ] T029-050 [US6] Explain endpoint (self or `access:read`) and `AuthContext.namespacePermissions`/`folderLevels`. Refs: FR-029-026. Evidence: T029-049.
- [ ] T029-051 [P] [US6] Studio Access screens in `frontend/apps/studio/src/features/access/` with component tests. Refs: FR-029-031. Evidence: `npm --prefix frontend run test`.
- [ ] T029-052 [US6] Studio folder tree, move action, preview dialog (with the warning that item names are not confidential) in `features/{workflows,scripts,connections}`; explicit confirmation before granting an administrator-equivalent permission; actions hidden per server permissions; regenerate `frontend/apps/studio/src/api/schema.d.ts`. Refs: FR-029-031. Evidence: typecheck, tests, `npm run check:api`.
- [ ] T029-053 [US6] Chromium journey and accessibility checks in `tests/e2e/custom-roles-folders/`. Refs: FR-029-031, NFR-029-ACC. Evidence: SC-029-010.

## Phase 9 — US07 Scripts and connections in folders (P3)

- [ ] T029-054 [P] [US7] Opacity battery and canary scan entries for script and connection operations (versions, test runs, executions, probe, ad hoc, results, upsert by name). Evidence: fails first.
- [ ] T029-055 [US7] Script placement, moves, filtered listings (`crates/application/src/scripts.rs`); enable the script kind gate. Refs: FR-029-011, FR-029-012, FR-029-017. Evidence: T029-054.
- [ ] T029-056 [US7] Connection placement and moves; admission and capability-use rechecks in `crates/infra-postgres/src/database/admission.rs`; non-admin connection writes need `secret:use` and the secret's list; enable the connection kind gate. Refs: FR-029-023, FR-029-036. Evidence: US07 scenarios; `integration_database_connections` as `sagat_app`.

## Phase 10 — US08 Identity-provider groups stay out (P3)

- [ ] T029-057 [P] [US8] Tests that token group and role claims confer nothing and that an external group is refused (`ACCESS_CAPABILITY_DISABLED`). Refs: FR-029-009. Evidence: integration test.
- [ ] T029-058 [US8] Hand `contracts/group-source.md` to the 013 owner as the dependency contract (documentation only, no schema). Refs: FR-029-009. Evidence: dependency note merged in 013.

## Phase 11 — Polish and release gates

- [ ] T029-059 Fault suite `tests/fault/custom-roles-folders/`: crash at each statement of every access change, create-in-folder, move, preview/apply and membership deletion; audit insert failure; store unavailable; corrupt rows; duplicates; restart. Refs: NFR-029-DUR, FR-029-027. Evidence: SC-029-007.
- [ ] T029-060 Race suite: revocation vs in-flight Query/Update, approval decision and disclosure; revocation vs signal/update; granter reduction vs grant. Refs: FR-029-021. Evidence: SC-029-006.
- [ ] T029-061 [P] Canary suite for logs, spans, metrics, audit and error bodies. Refs: FR-029-027, FR-029-029, NFR-029-OBS. Evidence: SC-029-012.
- [ ] T029-062 [P] Five-SDK scenario `tests/contract/sdk-conformance/scenarios/access-control.json` and reason handling in `sdks/{typescript,go,python,dotnet,java}`. Refs: FR-029-033. Evidence: `sdk_conformance` for all five (SC-029-009).
- [ ] T029-063 [P] Ceiling tests for every row of the plan's ceilings table. Refs: FR-029-030. Evidence: SC-029-011.
- [ ] T029-064 Benchmark `tests/performance/reports/custom-roles-folders.md`: environment, added latency per request (including the per-admission requirement read) and per filtered listing, fence waits with a long reader, the measured latency difference between hidden and missing targets (G1457), failures. Refs: NFR-029-PERF. Evidence: SC-029-013.
- [ ] T029-065 Compatibility gate: tenants without access objects unchanged except the declared `/version` carve-out; update `*.baseline.json`. Refs: FR-029-034, NFR-029-COMP. Evidence: SC-029-014.
- [ ] T029-066 Merge fragments into the canonical OpenAPI, manifest schema and remediation table; regenerate `docs/reference/{api,cli}.md`. Refs: FR-029-022, FR-029-028. Evidence: `contract_developer_experience`; CI `contracts`.
- [ ] T029-067 [P] Runbook `docs/runbooks/access-control.md` (delegation, restriction, preview, revocation, requirement and downgrade procedure, worker-binding caveat) and updates to `docs/runbooks/{security-governance,control-plane,studio}.md`, `docs/glossary.md`, `docs/architecture/security-and-sandbox.md`, cross-notes for G39/G49 in `specs/009-security-multitenancy/research.md`. Refs: Definition of Done. Evidence: documentation review.
- [ ] T029-068 Make `quickstart.md` executable as `e2e_custom_roles_folders quickstart::`, comparing CLI and REST answers. Refs: FR-029-032. Evidence: run log (SC-029-016).
- [ ] T029-069 Final gates: `cargo fmt`, `clippy -D warnings`, `check-architecture.py`, `check-agent-config.py`, runtime-role suites, workspace tests; `specs/029-custom-roles-folders/traceability.md` and `docs/traceability.md` with actual results, failures and skips. Refs: all. Evidence: recorded output.

## Dependencies and order

- T029-003 ships in an earlier release. Phase 2 blocks every story; T029-017 waits for T029-012; T029-022–024
  wait for T029-013/014/016.
- US01 → US02 → US03; US04 needs US01 and US03; US05 and restriction wait for the gate (T029-039); US07
  enables its kinds through its own battery entries; US08 is independent.

## Parallel opportunities

- Phase 2: T029-004, T029-008, T029-009, T029-010 in parallel; T029-005 then T029-013.
- Each story's test-writing task runs in parallel with the previous story's implementation.
- T029-025, T029-026, T029-040, T029-051, T029-061, T029-062, T029-063, T029-067 touch separate files.

## Implementation strategy

MVP = prerequisite release + Phase 2 + US01 (custom roles, fail-closed migration of call sites). Then groups
and open folders. Restriction and operation lists ship per item kind only after their gates. Each increment
carries its own evidence; an unchecked task is not a shipped guarantee.
