# Feature 029 — Baseline, sources, decisions, gaps and owner questions

Baseline: 2026-10-26, worktree `sagat-octopus-parity-specs` at `70f09435` (release 0.2.0). Nothing below is
implemented. Identifier range: D1440–D1479, G1440–G1479, O1440–O1479.

## Repository evidence (verified paths)

| Evidence | What it shows |
|---|---|
| `crates/domain/src/identity.rs` | 48 permissions (`Permission::ALL`); seven roles incl. `DatabaseRunner`; `ADMIN_PERMISSIONS` = all but `payload:reveal-pii`; `Principal(tenant, subject, role)`; `authorize` reads the role only. |
| `crates/infra-postgres/migrations/20261008000000_identity.up.sql`, `…000200_runtime_role_grants.up.sql`, `docs/runbooks/control-plane.md` §8 | `memberships` key `(tenant_id, subject)`, one role; `sagat_app` may only `SELECT` it; operators write members; revocation = delete the row, effective next request. |
| `…/20261126000000_authority_read_locks.up.sql` | `SECURITY DEFINER` lock functions (`sagat_lock_memberships`, …) let the runtime role fence revocation without `UPDATE` (017). |
| `…/20261202000000_reveal_grants.up.sql`, `…/20261220000000_reveal_grants_role_change.up.sql`, `crates/{application,infra-postgres}/src/reveal_grants.rs` | Grant-list precedent: cascade from memberships; removal on any role change (D663); current role re-read under lock; advisory lock exclusive on revoke, shared on use; cap 256; audit in the transaction. |
| `crates/infra-postgres/src/{declared_operations,approval_progress,event_payloads}.rs` | Store-side current-role rechecks inside decision transactions (G294 notes the runtime-role row-lock limit). |
| `crates/application/src/authz.rs` | Foreign namespace = `NotFound`; probe audited `access.denied.cross_tenant`. |
| `crates/application/src/**` | 154 `principal.authorize(Permission::…)` call sites in 25 files, all role-only. |
| `…/20261009000000_definitions.up.sql`, `…/20261014000000_scripts.up.sql`, `…/20261123000000_database_connections.up.sql`, `…/20261009000100_execution.up.sql` | Names unique per (tenant, namespace); no folder concept; `runs.workflow_id` exists. |
| `specs/001-platform-foundation/contracts/openapi.yaml` | 160 operations (153 `x-implemented`), no access extension; `AuthContext.role` enum of seven; closed `Error.code`, refinements in `details.reason`. |
| `tests/contract/security-multitenancy/rest/battery.rs`, `tests/integration/platform-foundation/runtime_privileges.rs` | D50 cross-tenant battery over every operation; D350 runtime-privilege sweep. Templates for the opacity battery and grants. |
| `specs/009-security-multitenancy/research.md`, `specs/016-payload-classification/research.md`, `specs/001-platform-foundation/research.md` | G39, G49, D43, D46, D50, D51, D52; G288, G290; G7 (no token revocation; roles read per request). |
| `docs/adr/ADR-0031-*.md`, `docs/adr/ADR-0036-*.md` | Per-name ACL needs "another decision"; 48 permissions and seven roles confirmed. |
| `specs/011-gitops-versioning/contracts/manifest.schema.json`, `specs/014-workflow-primitives/data-model.md` | Manifest entries closed (`additionalProperties: false`); ≤ 32 queries and 32 updates per workflow. |
| `…/sagat-octopus-orchestrator-013-federated-login/specs/013-federated-login/spec.md` | 013 draft: one role per session from memberships; groups/attributes from the provider do not grant; SCIM and group mapping out of scope. |

## External semantics consulted (2026-10-26)

- Temporal Cloud [custom roles](https://docs.temporal.io/cloud/manage-access/custom-roles) (pre-release,
  [changelog](https://temporal.io/changelog/custom-roles-pre-release)): permission sets for account and
  namespace resources, assignable to users, groups and service accounts; they **cannot narrow** the
  predefined role (union); every principal keeps a predefined role; delegating role creation or assignment
  is documented as an escalation risk; deleting a role revokes it. Small default limits
  ([limits](https://docs.temporal.io/cloud/limits); the pages disagree on 20 vs 25 permissions per role).
- Temporal Cloud [user groups](https://docs.temporal.io/cloud/manage-access/user-groups): local and SCIM
  groups coexist; SCIM groups change only through SCIM; most permissive grant wins; service accounts cannot
  join; only owners and global admins manage groups.
- Windmill [groups and folders](https://www.windmill.dev/docs/core_concepts/groups_and_folders) and
  [roles](https://www.windmill.dev/docs/core_concepts/roles_and_permissions): folders hold scripts, flows,
  resources and schedules with viewer/writer/admin for users and groups; no inheritance into subfolders;
  instance groups from SCIM; workspace admins bypass ACLs.

Taken: union with a base role, SCIM as a read-only group source, folder levels for users and groups, flat
inheritance. Not taken: admin bypass (D1450), scope inside role definitions (D1441), token claims (D1447).

## Decisions

- **D1440** Built-in roles stay (seven in code today; the input said six, G1440) as the floor; custom grants
  add. This feature removes or narrows no built-in role or grant (additive rule, D1655).
- **D1441** A role is owned by the tenant or one namespace; scope lives on the assignment, which keeps the
  non-escalation check computable for namespace delegates. Names are unique per owner, so a namespace
  delegate cannot probe other namespaces' role names; the scope of an existing object comes from its row.
- **D1442** Reserved: machine permissions (bindings own them), `payload:reveal-pii` (list, D275),
  `payload:read-internal` (ADR-0033 fixes its audience by built-in role, and the reveal grant requires it),
  `membership:manage` (manages that list), `group:manage` (a member addition grants a group's whole reach,
  too costly to bound, G1445), and in this increment `worker:bind` (routes secret pools and hidden workflows'
  inputs to a worker the holder controls) and `secret:manage` (rewrites any secret list). `quota:manage`,
  `egress:manage` and `audit:read` stay assignable but are marked administrator-equivalent (explicit
  confirmation on grant); `audit:read` sees past restriction. Every other built-in grant is a default, not a ceiling: ADR-0064 lists the
  decisions it refines (ADR-0018, 0028, 0031, 0035, 0036; FR-003-005; FR-014-007; D52; G49). The D46
  bypass of secret access lists stays tied to the built-in `admin` role, never to a permission.
- **D1443** Non-escalation is judged on committed state after the exclusive fence.
- **D1444** Assignments do not depend on their granter afterwards (as reveal grants and secret entries).
- **D1445** No deny rules; narrowing only by restricted folders and operation lists (deterministic,
  explainable).
- **D1446** Local groups are flat, human-only, administered by built-in `admin`.
- **D1447** No identity-provider group path is built here (AGENTS rule 2: no approved dependency). Token
  claims never authorize (tokens cannot be revoked before expiry, 001 G7); a future source writes durable
  rows under contracts/group-source.md (generations, base generation for deltas, heartbeats, staleness).
- **D1448** Folders are flat, per namespace, open by default; deletion only when empty.
- **D1449** Folder levels are fixed sets; sensitive permissions are never conferred by a level (see D1471).
- **D1450** No silent admin bypass, no lockout: `folder:manage` holders always administer entries (judged
  against their ceiling) and add themselves through an audited change.
- **D1451** A run follows its workflow's current folder (pinning would keep old runs visible).
- **D1452** Item names stay namespace-unique (folder-qualified names break references, triggers, manifests,
  SDK calls); a collision with a hidden item reveals only that the name is taken (G1441, D1474).
- **D1453** Operation lists are management rows keyed by name across versions, not IR (IR would need a
  republish to revoke); narrowing only; managed by `definition:write` on the workflow.
- **D1454** Coverage before restriction: `x-access` on every operation plus the opacity battery gate the
  restricted mode and lists (unmigrated routes would fail open); custom grants may ship earlier (they fail
  closed on unmigrated routes).
- **D1455** Revocation follows 009 D51 (no cache); sensitive store rechecks take shared tenant and namespace
  access fences that writers take exclusively; streams re-evaluate every 30 seconds at most.
- **D1456** Engine admissions that act for no recorded subject are not re-authorized by folders;
  publication checks visibility of references; connections keep 017's admission recheck because they carry
  credentials; actions on behalf of a recorded subject (018 schedules, 026 links) are rechecked (D1656).
- **D1457** Moves need folder administration of both ends (root = `folder:manage`) and `folder:manage` when
  the source is restricted; promotion out of a restricted folder names a destination and needs
  `folder:manage` for the root or an open folder there; manifest placement is a move (D1472).
- **D1458** A built-in role change revokes the subject's custom grants (as 016 D663; pending O1441).
- **D1459** Machine identities are never principals; with `worker:bind` reserved, G49 stays open (G1449).
- **D1460** New permissions `access:read`, `role:manage`, `role:assign`, `group:manage` (reserved),
  `folder:manage`, held by `admin` only by default. The classification is a `match` with explicit arms and a
  compile-time default arm of `reserved`, so a permission another feature adds compiles and stays reserved
  until classified (a contract test lists them); counts are derived.
- **D1461** No IR, worker-protocol or AsyncAPI change; nothing consumes access changes asynchronously.
- **D1462** The evaluator returns sources; `access:read` explains anyone, a member explains themselves; a
  hidden target is `404` for the explainer. Without `access:read`, an ineligible principal is reported as
  `PRINCIPAL_NOT_ELIGIBLE`, so entries and lists cannot enumerate members or machine identities.
- **D1463** Previews read without locks; apply carries the folder revision. A preview counts and identifies
  only items visible to the previewer and reports the rest as one opaque total, only to `folder:manage` or
  `access:read` holders (re-review S2).
- **D1464** Counts are quota resources: `custom_roles` and `access_groups` per tenant (no namespace level,
  as D42), `folders` per namespace; namespace roles are also bounded per namespace so one delegate cannot
  exhaust the tenant's roles; per-entity bounds are constants (ADR-0027).
- **D1465** Audit in the change transaction; hidden-item probes on the collapsing denial path (D50, D56).
- **D1466** The Studio renders server decisions and never evaluates rules (G1453).
- **D1467** Lock order: readers keep their existing prefix and take the shared fences right after their
  membership share locks; writers take membership share locks, the fences (with `lock_timeout`), then only
  access-table rows, pre-locking any foreign row they reference with `KEY SHARE NOWAIT` and retrying (≤ 3), so
  they never wait on a non-access row while holding a fence (publication locks the workflow before
  re-checking authority) and cannot stall readers indefinitely behind a queued exclusive request; proven by
  barrier tests (T029-012).
- **D1468** Role revisions are immutable history; assignments reference the role.
- **D1469** Deletes are refused while referenced (`ROLE_IN_USE`, `GROUP_IN_USE`, `FOLDER_NOT_EMPTY`).
- **D1470** Approval policies keep subjects and built-in roles (FR-014-007; G1448).
- **D1471** Levels are bounded by the built-in `developer` role and apply only to their kinds. `run:operate`,
  `run:signal`, `version:activate` and `database:manage` are sensitive: `database:manage` lets the writer pick
  a secret by name (`crates/infra-postgres/src/database.rs`), so a non-admin writer also needs `secret:use` and
  the secret's list (FR-029-036).
- **D1472** A move is a grant: the destination's entries must be within the mover's power on the item
  (`canPlace`); creating in a folder adopts its policy and needs only the create permission there.
- **D1473** Operation lists narrow freely (`definition:write`) but widening (adding principals, deleting the
  list) needs `role:assign` and the right to call the operation the actor would have without the list
  (being listed is not required, so an empty list cannot lock everyone out; the widening is audited). Adding a member to a group named
  by lists or restricted-folder entries is the same widening (`canAddMember`) and is audited as such, so an
  administrator joining a listed group cannot bypass the audit of a widening (re-review S1).
- **D1474** Decision order: refuse with `403` before any lookup when neither the ceiling nor any of the
  caller's levels could confer the permission on that kind (today's order, `crates/application/src/runs.rs`);
  then missing and hidden targets answer the same `404` (re-review B1: checking visibility first made `404`
  appear only for hidden items). Name-keyed writes and lookups (create, upsert by name, promotion target,
  manifest publish) and idempotent replays resolve visibility next; a hidden target is never read or written
  and answers as the operation's ordinary collision; rate admission charges hidden targets as missing ones.
- **D1475** Publication needs `run:start` on referenced children, `script:read` on pinned scripts and
  `database:use` on connections; a level-only publisher of a triggered workflow needs `event:manage` in the
  ceiling (triggers act namespace-wide; the publisher chooses the input classification, ADR-0033 item 1).
- **D1476** Folders have visibility (`visibleFolder`); a hidden folder answers as missing everywhere it is
  named. `visibleFolders(kind)` in the domain is the only implementation used by listings; SQL receives it as a
  parameter and never re-derives the rules (re-review S4).
- **D1477** Coverage is `x-access` with `discloses`, generated from typed handler extractors, plus a per-kind
  opacity battery and a canary scan of every response, header and rate effect; narrowing is enabled per kind.
- **D1478** Narrowing needs a binary guard: a requirement check on every API admission and stream
  re-evaluation (`503` `BINARY_CAPABILITY_MISSING`) and a capability heartbeat, deployed one release earlier;
  narrowing objects are refused until `access-narrowing-v1` is recorded, which is refused while a live
  instance lacks it (re-review B2: a readiness probe leaves a window, and Compose does not route by
  readiness). The mechanism is 018's registry (D1658); activation is an operator act (D1659).
- **D1479** Placement lives in `item_placements`, not on item rows: 017's identity trigger forbids such
  updates, tombstoned connections would block folder deletion, and item row locks contend with publication.

- **D1655** Built-in roles are additive (coordinator decision C10): other drafts add machine roles (021
  `integration-runner`, 025 `agent-runner`, 020's machine identity if adopted) and grants to existing roles
  (018, 019, …); 029 removes or narrows none, treats every role it does not list as human (`admin`,
  `developer`, `operator`, `viewer`) as a machine role that can never be a principal, and tests set properties,
  never counts.
- **D1656** Actions taken on behalf of a recorded subject are rechecked through the evaluator at fire or
  submit time instead of being exempt like engine internals: 018 schedule actions against `authorizedBy`
  (FR-018-015, pause with `AUTHORITY_REVOKED`) and 026 public and decision links against their owner. Without
  this, a subject removed from a restricted folder would keep a schedule firing. 020 list queries and saved
  views, 021 resources (which adopt folder placement), 025 MCP listing and calls and 026 links state that
  visibility is decided by this evaluator when 029 is enabled.
- **D1657** 029 follows ADR-0050 decision 7 (coordinator decision C4): the access-capability check is a read
  in slot 1 (with the 026 link / 027 app-session rows); memberships `FOR SHARE` → 029 access fences → namespace
  row are slot 4a, after the receipt (2), aggregates (3), existing runs (3R) and 017's catalog lock; T029-012
  proves it.
- **D1658** 029 uses feature 018's instance-capability registry (`specs/018-schedules-bulk-operations/
  data-model.md`, T018-080, 018 D1034) instead of its own `serving_instances` and requirement tables
  (coordinator alignment): one registry for every control-plane and engine process, shared with 031. 029 keeps
  its semantics — per-admission re-check, no narrowing while any live `CONTROL_PLANE` instance lacks
  `access-narrowing-v1`, the older-replica test — and owns only the capability value and the gate on its
  narrowing writes.
- **D1659** Activating `access-narrowing-v1` is an audited operator act through 018's registry command
  (coordinator decision; 018 amends FR-018-052 and registry rule 5), never a side effect of a tenant's first
  restricted folder: a deployment-wide requirement must not be switched on by one tenant. Until it is
  recorded, restricted folders and operation lists are refused with `ACCESS_CAPABILITY_DISABLED`; creation
  reads the epoch `FOR SHARE`, activation and removal lock it `FOR UPDATE`.

## Gaps

| # | Gap | Status |
|---|---|---|
| G1440 | Seven built-in roles and 48 permissions in code; ADR-0033, 016 and the input say six, 009 G39 says five (extends G288). | Open (documentation pass) |
| G1441 | A create or rename conflict reveals, inside the namespace, that a hidden item's name is taken; such a collision is audited as a probe and charged like any create; the Studio warns on restriction that names are not confidential. | Accepted (D1452) |
| G1442 | Quotas, alert counts and other aggregates include hidden items (counts only). | Accepted |
| G1443 | External groups need a source; 013 excludes SCIM and group mapping. | Open (O1440) |
| G1444 | No member-management surface (016 G290). | Open (O1442) |
| G1445 | No delegated group administration. | Open |
| G1446 | No nested folders or groups. | Accepted for this increment |
| G1447 | Published references to scripts and child workflows are not re-authorized after later folder changes (connections are). | Accepted (D1456) |
| G1448 | Approval eligibility cannot name groups or custom roles. | Open (014 follow-up) |
| G1449 | Built-in `admin` still holds `worker:bind` and `worker:execute`, and `worker:bind` is reserved here, so the bind/read separation of G49 is not delivered. | Open |
| G1450 | Secrets stay out of folders; their per-secret lists remain. | Accepted |
| G1451 | Audit reads are not filtered by folder; `audit:read` is delegable tenant-wide, marked administrator-equivalent and "sees past restriction"; the canary scan excludes audit endpoints for this reason. | Accepted (explicitly trusted readers) |
| G1452 | Evaluation and filtered-listing cost unmeasured. | Open (T029-064) |
| G1453 | A Studio view may offer an action the server then refuses after a concurrent change. | Accepted |
| G1454 | A request admitted before a revocation may finish; a stream may run up to 30 s after it. | Accepted, bounded |
| G1455 | 021 resources and 027 apps must adopt the folder contract in their features. | Open (dependency) |
| G1456 | A visible parent's journal and graph still carry a hidden child's run id and outcome (`child.finished`). | Accepted for this increment; follow-up |
| G1457 | Timing is not equalized between hidden and missing targets; the item and placement are read in one statement on both paths and probe audits leave the response path, and the remaining difference is measured and published (T029-064). | Accepted residual |
| G1458 | A preview does not detect group or assignment changes made before apply (folder revision only). | Accepted (advisory) |
| G1459 | Binaries older than 018's registry release neither register nor re-check, so they cannot be detected or made to refuse; the rollout must pass through that release (same as 018 G1607). | Accepted, runbook |
| G1460 | Restriction does not govern worker bindings: a worker bound to an activity receives its tasks' inputs, including for hidden workflows, and built-in administrators (the only `worker:bind` holders, since it is reserved here) can arrange that. | Accepted (API surfaces only) |
| G1461 | Permissions added by other drafts (018, 019, 020, 021, 022, 024, 025, 026, 027, 028, 030, 031, 032) and the machine roles 021, 025 (and possibly 020) add are reserved or machine-only until classified here; whichever feature merges second classifies them; 019's `push:manage` and `task:complete` should be at least sensitive. | Open (dependency) |

## Review findings (2026-10-26)

Independent read-only security and architecture reviews of the first draft found, among others: moves and
list widening were unchecked grants (D1472, D1473); name-keyed upserts and promotion bypassed opacity, and
rate admission was a side channel (D1474); the `editor` level conferred `database:manage`, which selects
secrets by name, plus operator-only powers (D1471, FR-029-036); child references and triggers widened
access through publication (D1475); folders themselves leaked (D1476); coverage could not see
namespace-class leaks (D1477); older binaries would fail open (D1478); placement on item rows conflicted with
017's trigger (D1479); the inventory of role-only checks was incomplete (plan, enforcement step 1); the
external-group schema had no approved dependency (D1447). Each fix is traced in spec, plan and tasks.

## Re-review findings (2026-10-27)

An independent architecture re-review found two blocking and four should-fix issues, all applied:
B1 decision order (D1474, vectors V29–V36, battery as every built-in human role); B2 binary guard as a
per-admission check plus heartbeat (D1478, FR-029-037, T029-003); S1 member addition as a grant (D1473,
`canAddMember`); S2 preview limited to visible items (D1463); S3 `worker:bind` and `secret:manage` reserved,
`quota:manage`/`egress:manage`/`audit:read` marked administrator-equivalent (D1442, G1449, G1451); S4 one
visibility implementation for listings (D1476). Minor findings: timing (G1457), name collisions (G1441),
audit exclusion from the canary scan (G1451), compile-time default of reserved (D1460), namespace-wide effects
of an `editor` (FR-029-013), and 019's routes in the adoption list (spec Dependencies). No new D/G/O number was
needed.

## Cross-feature consistency (2026-10-27, coordinator decisions C4 and C10)

Built-in roles are additive and tested by set properties (D1655); schedules and links are rechecked at fire
or submit time, and the adoption list names 020, 021, 025 and 026 (D1656); the access-capability check is
slot 1 (read) and slot 4a (memberships, fences, namespace row) of ADR-0050 decision 7 (D1657); the capability tables are 018's instance-capability
registry (D1658) and activated only by an operator (D1659). New numbers: D1655–D1659 (overflow range).

## Owner questions

- **O1440 — Source of identity-provider groups.** (A) amend 013 or a successor to push SCIM 2.0 Groups
  into `external` groups; (B) read group claims at sign-in; (C) defer. Recommended **A**: B cannot be
  revoked before token expiry and contradicts 013's assumption that access comes from memberships.
- **O1441 — Built-in role change and custom grants.** (A) revoke direct assignments, local group
  memberships and entries; (B) keep them. Recommended **A**, as D663 for reveal grants.
- **O1442 — Member management.** (A) leave to 013 or a 009 follow-up; (B) add it here. Recommended **A**.
