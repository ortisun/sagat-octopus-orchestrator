# Implementation Plan: Custom roles, groups, folders and per-operation access lists

**Branch**: `feature/029-custom-roles-folders` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

Status: Proposed — not started.

**Input**: `specs/029-custom-roles-folders/spec.md`; decisions in [research.md](./research.md); proposed
[ADR-0064](../../docs/adr/ADR-0064-access-grants-add-and-only-restricted-folders-and-operation-lists-narrow.md).
Revised on 2026-10-26 after independent security and architecture reviews, and on 2026-10-27 after an
architecture re-review (research.md, "Review findings" and "Re-review findings").

## Summary

Add one pure access evaluator to the domain and make every REST operation ask it instead of the built-in
role. Its inputs are durable rows: the membership's built-in role, custom-role assignments (direct and
through local groups, tenant-wide or per namespace), the item's placement and folder (mode and entries), and
an optional per-name operation list. Custom grants and folder levels add (levels never exceed the built-in
`developer` role); only a restricted folder and an operation list narrow. Administration (roles,
assignments, groups, folders, entries, moves, lists) is a set of small audited transactions behind tenant
and namespace access fences, each refusing a grant beyond the actor's power; moves and list widening count as
grants, and so does adding a member to a group. Decisions keep today's order (permission before lookup,
then visibility), so missing and hidden targets answer alike. Narrowing ships per item kind only after a
contract test proves every operation declares what it decides and discloses, an opacity battery (as every
built-in human role) and a canary scan prove hidden items never leak, and every live control-plane instance
reports the capability in feature 018's instance-capability registry (T018-080), deployed one release
earlier. No IR,
worker-protocol, AsyncAPI, run-transition or scheduler change.

## Technical Context

**Language/Version**: Rust 1.98.0 (`rust-toolchain.toml`); TypeScript Studio; five SDKs at their pinned
toolchains. No toolchain change.

**Primary Dependencies**: existing Axum, SQLx, Tokio, OTel crates. No new dependency.

**Storage**: PostgreSQL (authority). Ten new access tables, listed in [data-model.md](./data-model.md); the
capability tables are 018's instance-capability registry, cited not redefined. No existing table gains a
column.

**Testing**: domain unit and property tests; real PostgreSQL integration as `sagat_app`
(`SAGAT_TEST_AS_RUNTIME_ROLE=1`); REST contract, opacity battery and canary scan; barrier tests; fault
injection per statement; Chromium e2e; five-SDK conformance; benchmark.

**Target Platform**: existing single-region deployment (Compose, Helm). No new process.

**Project Type**: modular monolith with REST, CLI, Studio and SDKs.

**Performance Goals**: benchmark criteria only (NFR-029-PERF). No promise before measurement.

**Constraints**: fail closed; no cache unless versioned; tenant-keyed rows; ADR-0014 `begin_safely` for every
new transaction; audit in the same transaction; closed metric labels; runtime-role privileges only.

**Scale/Scope**: 5 new permissions (built-in roles are additive: other drafts add machine roles and grants,
so tests assert set properties, never counts); 34 new REST operations; `x-access` on every operation (160 today); every
role-only decision migrated: 154 `principal.authorize(Permission::…)` sites in 25 application files plus the
other role reads listed under "Enforcement migration".

## Architecture and crate placement

```text
crates/domain/src/access.rs                 # NEW: catalog classification (default reserved), FolderLevel,
                                            #      AccessFacts, Target, Decision, evaluate, can_grant, can_place
crates/domain/src/identity.rs               # +5 permissions; Principal carries tenantWide permissions
crates/domain/src/{secrets,execution/approval}.rs  # role reads routed through the evaluator (admin bypass of
                                            #      secret lists stays tied to built-in admin, D1442)
crates/domain/src/{audit,quota}.rs          # actions, resource types, quota resources
crates/application/src/access.rs            # NEW: AccessStore port, Authorizer, admin use cases
crates/application/src/**                   # call sites move to the Authorizer, module by module
crates/infra-postgres/src/access.rs         # NEW: fact query, fences, effective_permission_in_tx, listings
crates/infra-postgres/src/**                # store rechecks (inventory below) use effective_permission_in_tx
crates/infra-postgres/migrations/<ts>_access_control.{up,down}.sql
crates/api-http/src/routes/{access.rs,mod.rs}, dto_access.rs, error.rs       # typed access extractors
crates/telemetry/src/metrics.rs             # closed metric enums
apps/control-plane/src/                     # checks 018's registry requirement before narrowing writes
apps/cli/src/access.rs                      # NEW `sagat access ...`
scripts/check-architecture.py               # NEW rule: no role-only decision outside domain::access
frontend/apps/studio/src/features/access/   # NEW Access screens; folder tree in existing features
sdks/{typescript,go,python,dotnet,java}/    # reasons only; conformance scenario
tests/{integration,contract,fault,e2e}/custom-roles-folders/, tests/performance/reports/
```

The domain module holds only pure types and the evaluator (no SQLx, Axum or `serde_json`, constitution II).
The application `Authorizer` builds `AccessFacts` for (tenant, subject, namespace[, target]) from one store
call. HTTP DTOs exist only where the wire shape differs (principal discriminators, revision headers).

## Evaluation model

Normative in [contracts/evaluation.md](./contracts/evaluation.md): `tenantWide`, `ceiling(N)`, `levels(F)`,
`allowItem` (level permissions: ceiling or levels in open folders and at the root, levels only in restricted
folders; sensitive permissions: ceiling and visibility; a level applies only to its kinds),
`visibleFolder`, `visibleFolders(kind)`, `allowOperation`, the decision order (step 1: refuse with `403`
before any lookup unless the caller may hold the permission on that kind through ceiling or a level; step 2:
missing or hidden answer the same `404`; step 3: allow or `403`), and the administration rules `canGrant`,
`canPlace` (a move is a grant), `canCreateIn`, `canWidenList` and `canAddMember`. Every union commutes; the only conjunctions are the restricted branch, the
sensitive branch and the operation list. Levels are bounded by the built-in `developer` role, so a level
never hands out `run:operate`, `run:signal`, `version:activate` or `database:manage`. The evaluator returns
its sources; for the metric label the first contributing source in the order builtin, custom, folder is
counted.

## Enforcement migration

1. **Inventory and a check that keeps it complete.** Every role-only decision moves to the evaluator:
   the 154 `.authorize(Permission::…)` sites; the variable-permission calls in
   `crates/application/src/{api_admission,database_results,payloads,runs}.rs` and `runs/ad_hoc.rs`; route
   checks in `crates/api-http/src/routes/{outbound,runner}.rs`; direct role reads in
   `crates/domain/src/{secrets.rs,execution/approval.rs}`,
   `crates/application/src/{payload_disclosure,event_payloads}.rs` and
   `crates/infra-postgres/src/{declared_operations,approval_progress,event_payloads,payload_disclosure,
   managed_secrets,reveal_grants,database,database_results,secrets}.rs`, `database/admission.rs`. Machine
   checks (`workers.rs`, runner routes) keep their role and binding logic (D1459). A new rule in
   `scripts/check-architecture.py` fails on `Role::grants`, `Principal::authorize` or `== Role::` outside
   `domain::access` and an explicit allowlist of machine routes; `docs/engineering/architecture-principles.md`
   gains its row.
2. **Request-level checks.** `Principal` carries `tenantWide` computed at authentication. Namespace targets
   call `Authorizer::require(ns, permission)`; item targets `require_item(ns, kind, id, permission)`, which
   applies step 1 before touching the item, then resolves item and placement in one statement (the same
   statement for missing and hidden targets) and returns `NotFound` for both; listings pass the domain's
   `visibleFolders(kind)` set to SQL as a parameter, and no SQL predicate re-derives the rules (a property
   test checks listing ≡ `visible(i)`). Hidden-item probe audits go to the collapsing denial path off the
   response path. `Authorizer::recheck_for_subject(subject, ns, item, permission)` evaluates on behalf of a
   recorded subject; 018 schedule actions (against `authorizedBy`, FR-018-015) and 026 link submissions
   (against the link owner) call it at fire or submit time when 029 is enabled, and 020 list queries and
   saved views, 021 resources and 025 MCP tool listing and calls use the same evaluator. Rate admission (`api_admission.rs`) resolves visibility before charging, so a hidden
   target is charged as a missing one.
3. **Store rechecks** use `effective_permission_in_tx`: membership share lock (`sagat_lock_memberships`),
   shared tenant and namespace fences, fact read and evaluation on the transaction's own connection (D44).
4. **Coverage.** Each handler takes a typed access extractor (`TenantAccess`, `NamespaceAccess`,
   `ItemAccess<K>`, `FolderAccess`); the route registry is generated from the handler signatures and a test
   compares it with each operation's `x-access` (`class`, `permission`/`anyOf`, `kind`, `discloses`). A
   contract test fails when a response schema carries `workflowId`, `runId`, `scriptId`, a connection name or
   a folder without `discloses`.
5. **Gates per item kind.** `GET /version` reports `accessControl` per kind. Custom roles, groups and open
   folders enable when their slice gates pass (an unmigrated route ignores a custom grant: fail closed).
   Restricted mode and lists enable for a kind only after (a) its opacity battery over every operation that
   names or discloses the kind, as every built-in human role and a level-only member, (b) a canary scan of
   every operation's responses, headers and rate effects for a hidden canary item (audit reads excluded,
   G1451), and (c) 018's instance-capability registry in service fleet-wide with `access-narrowing-v1`
   activated by an operator (FR-029-037).

## Transaction boundaries

All new transactions start with `begin_safely` (ADR-0014). "Fence" means the access fences below.

| Operation | Commits together | Fences / notes |
|---|---|---|
| Create/edit/delete custom role | role row, revision, quota charge, idempotency receipt when a key is given, audit | tenant role: tenant exclusive; namespace role: tenant shared + namespace exclusive; non-escalation on committed power; delete checks no assignment |
| Assign / unassign | assignment row, idempotency receipt when a key is given, audit | fence by stored scope; subject membership share-locked and human-checked |
| Group create/edit/delete, member add/remove | group / member rows, quota charge, audit; a member addition also audits a widening of each list and restricted-folder entry naming the group | tenant exclusive; `canAddMember` on committed state; delete checks references |
| Folder create/rename/mode/delete | folder row + revision, quota charge, audit (+ entries and tombstoned placements on delete); a restriction requires `access-narrowing-v1` already recorded by the operator (epoch read `FOR SHARE`) | tenant shared + namespace exclusive, then the epoch row; `ACCESS_CAPABILITY_DISABLED` until activated |
| Folder entry set/remove | entry row, folder revision, audit | as above; non-escalation |
| Create an item in a folder | the existing create transaction plus the placement row, folder revision, audit | adds tenant shared + namespace exclusive before the placement; creation at the root is unchanged |
| Move item (including manifest placement) | placement upsert/delete, both folder revisions, audit | item `KEY SHARE NOWAIT` before the placement; `canPlace` on committed state |
| Operation list create/narrow/widen/delete | list and principals rows, audit; creating a list requires the recorded requirement the same way | tenant shared + namespace exclusive, then the epoch row; workflow `KEY SHARE NOWAIT` |
| Membership role change / deletion (operator) | cascade/trigger removals and `access.revoked_on_role_change` audit | existing operator transaction; `SECURITY DEFINER` trigger as in `20261220000000` |
| Sensitive decision (store rechecks) | the existing decision and its existing audit | adds shared fences and the evaluator read |
| Preview | nothing (read-only snapshot) | returns folder revision; apply must match; advisory for group and assignment changes (G1458) |

No outbox, journal, timer or subscription row is written; nothing consumes access changes asynchronously.

### Fences and lock order

Fences are transaction advisory locks `access:t:{tenant}` and `access:n:{tenant}:{namespace}`
(`pg_advisory_xact_lock[_shared](hashtextextended(key, 0))`, as `reveal_grants.rs` does).

- **Admission slots (ADR-0050 decision 7, coordinator decision C4).** 029 appears twice. In slot 1, with the
  026 public-link row or 027 app-session row, only the access-capability check runs (018 registry epoch: a
  read, no lock). In slot 4a, after the request receipt (2), run-creating aggregates (3) and existing runs
  (3R), and after 017's namespace secret/catalog lock, 021's variable fence and 024's data-catalog fence,
  come memberships `FOR SHARE` (`sagat_lock_memberships`) → the two shared 029 access fences → the namespace
  row; slot 4b (catalog heads), 5 (secrets), 6 (new runs), 7 (022 semaphores) and 8 (bookkeeping, usage)
  follow. This matches the implemented 017 order (catalog lock before memberships).
- **Readers** that are not admissions (transitions on an existing run) keep their ADR-0050 order (run row
  first, 014's chain before run) and take both shared fences immediately after the membership share locks of
  that path. They read access tables without row locks.
- **Writers** set `lock_timeout` (2 s) for fence acquisition, take membership share locks, the fences, then
  only access-table rows. A foreign row they reference (item of a placement, workflow of a list) is pre-locked
  `KEY SHARE NOWAIT`; a failure rolls back and retries (3 attempts, bounded backoff), then answers `503`
  `ACCESS_WRITE_CONTENDED` (retryable). A writer never waits on a non-access row while holding a fence, which
  removes the cycle with paths that lock an item before re-checking authority (publication). Because a queued
  exclusive request makes new shared requests wait in PostgreSQL, the timeout also bounds how long a writer
  can stall readers; `access_fence_wait_seconds` measures it.
- A transaction that creates a restricted folder or an operation list reads 018's
  `platform_requirement_epoch` row `FOR SHARE` after its fences and refuses unless `access-narrowing-v1` is
  recorded; the operator's activation and removal commands lock that row `FOR UPDATE` and take no access
  fence, so they serialize with folder and list creation without a cycle.
- The lock-order inventory of every caller of `effective_permission_in_tx`, item creation in a folder and
  publication is T029-012, with barrier tests (move vs publish, move vs connection admission, folder delete
  vs item creation, long reader vs queued writer) before any store recheck is switched.

## Authorization matrix (administration)

| Operation | Needs | Non-escalation against |
|---|---|---|
| Read the permission catalog | any authenticated member | — |
| List/read tenant roles, tenant assignments, groups, entries by granter | `access:read` tenant-wide | — |
| List/read namespace roles and assignments in N | `access:read` in N (scope of the stored row) | — |
| Create/edit/delete a tenant role | `role:manage` tenant-wide | `tenantWide(actor)` |
| Create/edit/delete a namespace role of N | `role:manage` in N | `ceiling(actor, N)` |
| Assign/unassign at tenant scope | `role:assign` tenant-wide | `tenantWide(actor)` (assign only) |
| Assign/unassign in N | `role:assign` in N | `ceiling(actor, N)` (assign only) |
| Groups and members | `group:manage` (reserved: built-in `admin`) | member addition: `canAddMember` (widening of every list and restricted entry naming the group) |
| Grant an administrator-equivalent permission (`quota:manage`, `egress:manage`, `audit:read`) | as for any assignment, plus an explicit confirmation field | as for any assignment |
| Read a folder | `visibleFolder` | — |
| Create/rename/delete folders, change mode in N | `folder:manage` in N | — (mode change previewed) |
| Preview a restricting change on F | `folder:manage` (restrict, move-in) or `admin(actor, F)` (entry changes) | counts and identifies only items the previewer can see; one opaque total of the rest for `folder:manage`/`access:read` holders |
| Set/remove an entry on folder F | `admin(actor, F)` | `ceiling(actor,N) ∪ levels(actor,F)` |
| Create an item in F | the create permission allowed in F | — (adopts F's policy) |
| Move an item S → D (API or manifest) | `canPlace` | destination entries ⊆ `powerOn(actor, item)` |
| Create a list, remove principals | `definition:write` on the workflow | — (narrowing) |
| Add principals, delete a list | `definition:write` + `role:assign` in N + the right to call it without the list | `canWidenList` (no list can lock everyone out) |
| Read operation lists | `definition:write` on the workflow, or `access:read` in N | — |
| Write a connection (non-admin) | `database:manage` (sensitive) + `secret:use` + the secret's list | — (FR-029-036) |
| Explain (subject, permission, target) | own subject, or `access:read` in the target's scope | explainer must see the target |
| Auth context for a namespace | member | — |

Default built-in grants: `admin` gains the five new permissions; no other built-in role changes.

## Finite ceilings

| Resource | Default | Hard ceiling | Enforced |
|---|---:|---:|---|
| Custom roles per tenant, quota `custom_roles` (min of request, tenant setting, operator ceiling) | 100 | 500 | create, tenant fence |
| Namespace roles per namespace | 20 | 100 | create, namespace fence |
| Permissions per role | all assignable (43 today) | number of assignable permissions (derived; the wire schema caps the list at 64) | definition |
| Name length (role, group, folder; slug) | 3–63 | 63 | definition |
| Description (role, group, folder), UTF-8 bytes | 512 | 512 | definition; `content` |
| Folder display name, UTF-8 bytes | 128 | 128 | definition; `content` |
| Revisions per role | 1,000 | 1,000 | edit refused beyond |
| Direct assignments per subject | 32 | 64 | assign |
| Assignments per group | 64 | 128 | assign |
| Assignments per tenant | 5,000 | 20,000 | assign |
| Groups per tenant, quota `access_groups` (tenant formula) | 200 | 1,000 | create |
| Members per group | 1,000 | 5,000 | add |
| Groups per subject | 32 | 64 | add (bounds evaluation) |
| Folders per namespace, quota `folders` (namespace formula) | 200 | 1,000 | create |
| Entries per folder | 64 | 256 | set |
| Operation lists per workflow | 64 | 64 (32 queries + 32 updates) | create |
| Principals per operation list | 32 | 128 | widen |
| Preview sample identifiers (`access:read` holders only; counts for all) | 50 | 100 | preview |
| Stream re-evaluation interval | 30 s | 30 s (deployment may lower to 5 s) | stream |
| Writer fence `lock_timeout` / `NOWAIT` retries | 2 s / 3 | 5 s / 3 | writer |
| Folder entries and list principals naming one group | 128 | 256 | entry and list writes |
| Registry heartbeat / liveness / rows | 018's values (10 s / 30 s / 1,000) | 018's | 018 registry |

Zero refuses new creation; a lowered limit keeps existing rows usable and removable. One evaluation reads at
most 64 groups and 64 + 64 × 128 assignment rows, aggregated to at most 500 role permission sets; the
benchmark (T029-064) decides whether these limits are published as measured.

## Failure and recovery matrix

| Failure (rule 9) | Expected behaviour | Test |
|---|---|---|
| Crash at each statement of an access change, create-in-folder, move, preview/apply | change and audit both present, or both absent | fault per statement (T029-059) |
| Lost response, duplicate request | key replays the original result; changed body `IDEMPOTENCY_KEY_REUSED`; a replay naming a hidden item is refused | integration (T029-034) |
| Audit insert fails | whole change rolls back; nothing authorized | fault (T029-059) |
| Concurrent reduction of a granter's power vs their grant | fence serializes; one consistent order | barrier (T029-043) |
| Concurrent edits of one folder or role | revision CAS, `REVISION_CONFLICT` | integration |
| Move or list write vs a publish or admission holding the item | `NOWAIT` fails, ≤ 3 retries, then `ACCESS_WRITE_CONTENDED`; no deadlock | barrier (T029-012) |
| Long sensitive reader vs queued tenant writer | writer times out after `lock_timeout`; readers resume | barrier, benchmark |
| Revocation vs in-flight Query/Update, approval, disclosure; revocation vs signal/update | first commit wins; the loser is refused and audited | barrier (T029-060) |
| Membership deleted or role changed during an admin write | cascade or `UNKNOWN_SUBJECT`/`PRINCIPAL_NOT_ELIGIBLE`; no orphan | integration |
| Store unavailable or slow during evaluation | `UNAVAILABLE` (retryable); never allow | fault (T029-059) |
| Unknown permission or level in a stored row | that source contributes nothing; logged without values; never allow | unit + fault (T029-059) |
| Stream open across revocation | ends within 30 s; resume `404` | e2e (T029-038) |
| Older binary in service while restriction is requested | through 018's registry: the operator's activation is refused while any live control-plane instance lacks the capability, and narrowing objects are refused until activation; once recorded, a lacking registry-aware binary answers `503` on its next admission and stream re-evaluation, including after a pause beyond the liveness window; binaries older than the registry release do not register and are not protected (G1459, 018 G1607) | integration with a live older replica (T029-003), runbook |
| Downgrade with restricted folders or lists present | refused by readiness; remove all restriction and lists, then the operator clears the requirement; the down migration still refuses while access rows exist | runbook, integration |
| Lost broker delivery | not applicable: access changes publish nothing | run suites regression |
| Failed compensation | not applicable: no external effect; a connection moved mid-attempt keeps 017 in-flight semantics | 017 regression |
| Restart | no in-memory state; identical decisions before and after | restart test (T029-059) |

## Observability

- **Metrics** (closed labels): `access_decisions_total{class=tenant|namespace|item|folder, outcome=allowed|denied|hidden, source=builtin|custom|folder|none}`,
  `access_decision_duration_seconds{class}`, `access_fence_wait_seconds{side=reader|writer}`,
  `access_changes_total{kind=role|assignment|group|group_member|folder|folder_entry|placement|operation_list|role_change_revocation}`,
  `access_refusals_total{reason=escalation|reserved|scope_mismatch|in_use|limit|not_eligible|capability_disabled|operation_not_granted|contended}`,
  `access_stream_closures_total`; registry refusals are counted by 018's registry. No tenant, subject,
  role, group, folder or item identifier is a label.
- **Spans**: request span attributes `sagat.access.class`, `sagat.access.outcome`; administration span
  `access.change` with `sagat.access.change.kind` and the changed resource's UUID; never a subject.
- **Logs**: structured, redacted; never descriptions, display names or explanation sources by name.
- **Audit actions**: listed in [contracts/README.md](./contracts/README.md); existing
  `access.denied.permission` and `access.denied.cross_tenant` are reused.

## Migration, compatibility and rollback

- **Prerequisite release** (before 029): feature 018's instance-capability registry (T018-080: registration
  before serving, heartbeat, per-admission and per-stream re-check, activation) deployed fleet-wide; T029-003
  is the vehicle; 029 adds only the refusal of narrowing writes until an operator activates `access-narrowing-v1`. Upgrades must pass through it before restriction is enabled;
  Compose, which does not route by readiness, is covered by the per-admission re-check.
- **029 migration**: additive, timestamp allocated at implementation (after `20261220000000`); tables of
  [data-model.md](./data-model.md), `UNIQUE (tenant_id, namespace_id, id)` on `scripts`, quota resources,
  role-change trigger, `sagat_app` grants, classification entries. Down refuses while access rows exist.
- **Contracts**: OpenAPI grows additively (new paths, optional `folder` omitted when absent, `namespaceId`
  on auth context, `accessControl` on `/version`, recorded in the `*.baseline.json` compatibility files);
  every operation gains `x-access`, which clients ignore. The manifest gains an optional `folder`; an older CLI
  rejects manifests that use it.
- **Rolling upgrade**: during the rollout, old replicas ignore custom grants and folder levels (fail closed)
  and must never see a restriction, which the requirement check guarantees for guard-aware binaries.
- **Rollback**: disable creation and narrowing per kind; existing grants keep working. Binary downgrade
  follows the failure-matrix row.

## Constitution Check

Design checks, not implementation approval.

| Principle | Before research | After design | Evidence / gate |
|---|---|---|---|
| I — durable atomic transitions | Pass | Pass | No run transition changes; each change commits with its audit; fences serialize decisions (T029-012, T029-059, T029-060) |
| II — pure domain, inward deps, no empty mappers | Pass | Pass | Pure `domain/src/access.rs`; one store call per decision; new architecture rule (T029-005, T029-016) |
| III — at-least-once effects, honest compensation | Pass | Pass | No external effect; connection writes need secret authority (T029-056) |
| IV — pinned versions, compatible contracts, five SDKs | Pass | Pass | No IR/protocol change; additive REST/manifest with declared carve-out (T029-062, T029-065, T029-066) |
| V — isolation, tenant boundary, secrets, fail closed | Pass | Justified | Fails closed in this design (`worker:bind`, `secret:manage` reserved); pre-guard binaries cannot be protected, so restriction needs the prerequisite release (Complexity Tracking; T029-003, T029-026, T029-036, T029-039, T029-042) |
| VI — OTel, audit, typed errors | Pass | Pass | Closed metrics, audit per change, reasons with remediation, canaries (T029-008, T029-009, T029-061) |
| VII — inbox/outbox, redeliverable events | Pass | Pass | Nothing published; external group deliveries left to their source feature with an inbox-style contract |
| VIII — failure tests, honest contracts | Pass | Pass | Failure matrix; per-kind capability gates instead of unproven restriction (T029-039, T029-059, T029-060) |

## Complexity Tracking

| Choice | Why needed | Simpler alternative rejected because |
|---|---|---|
| Restricted folders replace the ceiling for level permissions | Folders must hide work from namespace-wide roles | Additive-only folders hide nothing; deny rules are unexplainable (D1445) |
| Per-kind gates on `x-access`, battery and canary scan | Every role-only route would fail open under restriction | Careful migration without a contract check repeats the risk D50 removed for tenants |
| Depending on 018's instance-capability registry, shipped one release earlier | Binaries that predate restriction would ignore it during a rollout or downgrade; a readiness probe alone leaves a window and Compose ignores readiness | A 029-only table would duplicate the registry 018 and 031 share; documenting "upgrade every replica first" is not enforceable |
| Placement in its own table | Item tables carry immutability triggers and row locks used by publication and admission | A column on items breaks 017's identity trigger and contends with publish locks |

## Delivery sequence

1. Prerequisite release: 018's instance-capability registry (T018-080) fleet-wide.
2. Foundational: classification, evaluator and vectors; schema, fences, store, fact query; inventory and
   architecture rule; lock-order proofs; errors, audit, metrics, quotas.
3. US01 custom roles and assignments, with module-by-module call-site migration.
4. US02 groups. 5. US03 open folders, placement, moves, coverage, battery, scan, then restriction for
   workflows and runs. 6. US04 delegation matrix. 7. US05 operation lists. 8. US06 explanation, auth
   context, Studio. 9. US07 scripts and connections (their own kind gates). 10. US08 claim refusal and the
   contract hand-off. 11. Gates: SDKs, benchmark, contracts merge, runbook, docs, traceability.

## Verification plan

Suites (new targets in `tests/Cargo.toml`): `integration_custom_roles_folders`,
`contract_rest_custom_roles_folders`, `fault_custom_roles_folders`, `e2e_custom_roles_folders`,
`perf_custom_roles_folders`; shared scenario `tests/contract/sdk-conformance/scenarios/access-control.json`;
regressions `integration_security_multitenancy`, `contract_rest_security_multitenancy` (battery),
`integration_platform_foundation` (runtime privileges, migrations), `integration_workflow_primitives`,
`integration_database_connections`, `integration_payload_classification`, `integration_gitops_versioning`,
`sdk_conformance`, with `SAGAT_TEST_AS_RUNTIME_ROLE=1` where supported; `python3 scripts/check-architecture.py`;
Studio `npm --prefix frontend run typecheck|test|build` and the Chromium journey. Results are recorded as
run, with failures and skips; nothing in this plan is evidence.
