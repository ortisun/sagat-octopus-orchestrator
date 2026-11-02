# Feature Specification: Custom roles, groups, folders and per-operation access lists

**Feature Branch**: `feature/029-custom-roles-folders`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Custom roles (Temporal Cloud 2026): tenant- or namespace-defined roles as named sets of catalog
permissions; the built-in roles remain; a granter cannot grant beyond their own permissions; assignment
to members and groups; audit; deterministic effective-permission evaluation; revocation on the next
admitted call. Folders inside a namespace for workflows, scripts, resources and apps, with ACLs
(owner/editor/runner/viewer) for users and groups; an optional per-name operation ACL (ADR-0031
currently grants namespace-wide). Groups: local groups now; identity-provider/SCIM groups from the 013
draft as a dependency.

## Context and delivery boundary

Today a person's access is one built-in role per tenant (`memberships`, key `(tenant_id, subject)`),
granting the same permissions in every namespace (`crates/domain/src/identity.rs`). Narrower control
exists only in resource lists: worker bindings (009 D43), a secret's access list (D46) and pools (G48),
and the `payload:reveal-pii` grant list (016 D275). Feature 009 accepted "no custom roles" (G39) and noted
that separating the power to bind workers from reading secrets needs them (G49); this increment keeps
`worker:bind` and `secret:manage` reserved, so G49 stays open (G1449). ADR-0031 grants
`run:query`/`run:update` for every declared operation of a namespace and leaves a narrower audience to "a
future authorization decision". This feature is that decision. It adds:

1. **Custom roles**: named sets of assignable permissions, owned by the tenant or one namespace, assigned
   to members or groups, tenant-wide or in one namespace. They only add to the built-in role.
2. **Local groups** of tenant members. Identity-provider groups are specified as a contract only and stay
   out until a source feature exists (013 excludes SCIM and group mapping today).
3. **Folders**: flat containers inside a namespace for workflows (with their runs), scripts and database
   connections, with an access list of fixed levels bounded by the built-in `developer` role. `open` by
   default (organizes, may add access); `restricted` on request, after which only the list decides who
   sees and acts on what is inside.
4. **Operation lists**: an optional list of callers for one declared Query or Update name of a workflow,
   narrowing ADR-0031's namespace-wide grant for that name.

One pure evaluation function decides all four from durable rows, and every REST operation declares which
decision it needs and which items its answers can disclose. A restriction hides items from the platform's
API surfaces (REST, CLI, Studio, SDK clients); it does not govern worker bindings (a bound worker still
receives the tasks it serves) nor audit reads (G1451, G1460); `audit:read` is marked as seeing past
restriction. No change to the workflow IR, the worker
protocol, AsyncAPI, run transitions, the scheduler, external effects or ADR-0033 audiences. Nothing here is
implemented.

**Ownership boundaries.** Identity-provider groups (claims or SCIM) belong to feature 013 (draft, sibling
worktree), whose Out of Scope lists SCIM and provider-group mapping; this feature only states the contract
a source must meet (O1440). Member creation and removal have no surface today (G290); this feature consumes
memberships as they are (O1442). Integration resources (021, which adopts folder placement for resources)
and apps (027) adopt the folder contract in their own features. Approval eligibility (FR-014-007) keeps
naming subjects and built-in roles (G1448). Built-in roles are additive: other drafts add machine roles
(021 `integration-runner`, 025 `agent-runner`, 020's machine identity if adopted) and add grants to existing
roles; permissions added by 018, 019, 020, 021, 022, 024, 025, 026, 027, 028, 030, 031, 032 are reserved until classified in this feature's catalog (G1461).

## Domain vocabulary

- **Built-in role**: one per membership; seven in code today: `admin`, `developer`, `operator`, `viewer`
  (human) and `worker`, `runner`, `database-runner` (machine). The set is additive: other features add
  machine roles and grants; this feature removes or narrows none. Only the four human roles can be principals.
- **Permission catalog**: 48 permissions today, 53 with this feature, more as other drafts merge; each is `assignable` or `reserved`,
  meaningful at `tenant` and/or `namespace` scope, and possibly an **item permission** for given item
  kinds, either a **level** permission (a folder level may confer it) or a **sensitive** one (never).
- **Custom role**: tenant role or namespace role; a permission set with a revision.
- **Assignment**: (role, principal, scope); principal = human subject or local group; scope = tenant or
  one namespace.
- **Namespace ceiling**: built-in role permissions plus those of every custom role assigned to the subject
  or one of their groups at tenant scope or in that namespace.
- **Folder**: named, flat, inside one namespace; `open` or `restricted`; has entries (principal, level) and
  a revision. **Placement**: the record that puts an item in a folder (no placement = namespace **root**).
- **Item**: a workflow (drafts, versions, publication requests, activations, runs), a script (versions, test
  runs, executions) or a database connection.
- **Folder level**: `viewer` ⊂ `runner` ⊂ `editor`; `owner` = `editor` plus administration of the folder's
  entries (FR-029-013).
- **Operation list**: callers allowed to invoke one (workflow, `query`|`update`, name).
- **Access fence**: transaction locks that make an access change and a sensitive decision serialize.

## User Scenarios & Testing *(mandatory)*

### US01 — Define a custom role and assign it in one namespace (Priority: P1)

An administrator defines `release-manager` (read definitions, activate versions, read and operate runs)
and assigns it to a `viewer` in namespace `payments` only.

**Why this priority**: the capability G39 deferred; every other story uses the evaluator it introduces.

**Independent Test**: define, assign in one namespace, call `activateVersion` in two namespaces, unassign,
call again; compare answers and audit.

**Acceptance Scenarios**:

1. **Given** the assignment, **When** the viewer activates a version in `payments`, **Then** it succeeds
   and is audited; in `orders` it is `403 FORBIDDEN`.
2. **Given** a definition listing `payload:reveal-pii`, `payload:read-internal`, `worker:execute` or
   `membership:manage`, **When** it is created, **Then** `422` `RESERVED_PERMISSION` and nothing is stored.
3. **Given** the assignment is removed, **When** the viewer's next request is admitted after that commit,
   **Then** it is refused; a request admitted before the commit may finish.
4. **Given** the role is edited to drop `run:operate`, **When** the viewer next cancels a run, **Then** it is
   refused; the previous revision stays in the role's history.
5. **Given** a role still assigned, **When** it is deleted, **Then** `409 CONFLICT` `ROLE_IN_USE`.
6. **Given** a worker or runner identity, **When** a role is assigned to it, **Then** it is refused
   (`MACHINE_IDENTITY` for an `access:read` holder, `PRINCIPAL_NOT_ELIGIBLE` otherwise).

### US02 — Grant access through local groups (Priority: P1)

An administrator creates `payments-oncall`, adds three members and assigns `release-manager` to the group
in `payments`.

**Why this priority**: per-person assignment does not scale and is how access drifts.

**Independent Test**: create a group, add members, assign to it, remove a member, delete a membership.

**Acceptance Scenarios**:

1. **Given** a member of two groups with overlapping roles, **When** evaluated, **Then** access is the
   union, independent of the order of groups or assignments.
2. **Given** a member removed from the group, **When** their next request is admitted, **Then** the group's
   grants no longer apply; removal and audit commit together.
3. **Given** a membership deleted or its built-in role changed by an operator, **When** that commits,
   **Then** the subject's group memberships, assignments and list entries go in the same transaction.
4. **Given** a group referenced by an assignment, entry or list, **When** deleted, **Then** `GROUP_IN_USE`.
5. **Given** a group, role, folder or subject identifier of another tenant, **When** used anywhere,
   **Then** the answer equals the one for a missing identifier and the probe is audited.
6. **Given** a group named by an operation list or by an entry of a restricted folder, **When** an
   administrator adds a member, **Then** it is checked and audited as a widening of each list and entry the
   group reaches, not only as a group change.

### US03 — Organize workflows into folders and restrict one (Priority: P1)

Folders `billing` (open; contractors get `runner`) and `payroll` (restricted; only its list sees inside)
in `payments`.

**Why this priority**: namespace-wide visibility is the main reason teams ask for folders.

**Independent Test**: place workflows, then read, list, write by name and replay keys as listed and
unlisted callers of every built-in human role and a level-only member, comparing every answer with a
nonexistent target, before and after restricting.

**Acceptance Scenarios**:

1. **Given** an open folder whose list gives a viewer `runner`, **When** the viewer starts a run of a
   workflow in it, **Then** it succeeds; at the root it is `403 FORBIDDEN`.
2. **Given** a restricted folder, **When** an unlisted developer lists or reads the folder, its workflows,
   versions, runs, journal, graph, stream, telemetry, approvals or payloads, **Then** lists omit them and
   every direct read is byte-identical to a missing resource.
3. **Given** a restricted folder, **When** a developer listed as `viewer` publishes, **Then** `403`.
4. **Given** an administrator not on a restricted list, **When** they read its items, **Then** they are
   hidden; the administrator may add themselves, audited, before anything becomes visible.
5. **Given** a proposed restriction, **When** previewed, **Then** the platform reports how many subjects and
   groups lose visibility, how many pending approvals, active runs and dependent published parents are
   affected, and applying it requires the folder revision the preview returned.
6. **Given** a non-empty folder, **When** deleted, **Then** `409 CONFLICT` `FOLDER_NOT_EMPTY`.
7. **Given** a hidden workflow or connection, **When** someone creates, upserts or promotes into its name,
   **Then** the answer is the operation's ordinary name collision (`409`; `NAME_UNAVAILABLE` where the
   operation had none) and the hidden item is neither read nor written (G1441).
8. **Given** a run started with an idempotency key on a workflow that later became hidden, **When** an
   unlisted caller replays that key, **Then** the stored response is not returned (`IDEMPOTENCY_KEY_REUSED`).
9. **Given** a viewer with no folder entries, **When** they cancel a hidden run or a nonexistent one,
   **Then** both answers are `403 FORBIDDEN`, before any lookup, as today; an operator gets `404` for both.
10. **Given** an 018 schedule or a 026 link whose recorded subject is removed from the restricted folder of
    its target workflow, **When** the schedule fires or the link is submitted, **Then** no run starts (the
    schedule pauses with `AUTHORITY_REVOKED`; the link refuses) and the refusal is audited.

### US04 — Delegate administration without privilege escalation (Priority: P2)

A team lead receives `payments-admin` (`role:assign`, `role:manage`, `folder:manage`) in `payments` and
administers access there without ever granting more than they hold.

**Why this priority**: delegation makes custom roles useful in large tenants; escalation is its main risk.

**Independent Test**: the escalation matrix of granter sets × grant kinds (roles, assignments, entries,
moves, list widening) × scopes (contracts/conformance.md).

**Acceptance Scenarios**:

1. **Given** a lead without `run:operate` in `payments`, **When** they define or assign a role containing
   it, **Then** `403 FORBIDDEN` `PRIVILEGE_ESCALATION` and nothing is stored.
2. **Given** authority only in `payments`, **When** the lead assigns in `orders` or at tenant scope, or
   edits a tenant role, **Then** `PRIVILEGE_ESCALATION`.
3. **Given** a folder owner whose built-in role is `viewer`, **When** they grant `editor` there, **Then** it
   succeeds; on another folder it fails.
4. **Given** a lead who lacks `run:start` on a root workflow, **When** they move it into a folder whose
   entries give a group `runner`, **Then** `PRIVILEGE_ESCALATION` (a move is a grant).
5. **Given** the lead's authority is removed, **When** assignments the lead made are evaluated, **Then** they
   remain until removed and name their granter (D1444).
6. **Given** a concurrent removal of the lead's authority and the lead's assignment, **When** both commit,
   **Then** either the assignment committed first, or it was judged against the reduced authority.

### US05 — Restrict a declared Query or Update to named callers (Priority: P2)

Only group `payroll-approvers` may call `payroll-run`'s Update `approveOverride`; every operator may still
call its Query `status`.

**Why this priority**: ADR-0031 discloses to every `run:query` holder; some operations are more sensitive.

**Independent Test**: set a list on one name; call it and another name, run- and chain-addressed, as
listed and unlisted callers; replay a receipt after revocation; try to widen the list without authority.

**Acceptance Scenarios**:

1. **Given** a list on `update/approveOverride`, **When** an unlisted operator calls it, **Then** `403`
   `OPERATION_NOT_GRANTED`, no receipt, audited; `status` works as before.
2. **Given** a listed caller, **When** they call it, **Then** they still need `run:update` on the run.
3. **Given** a receipt applied before the caller was removed, **When** replayed with the same key, **Then**
   it is refused (current authorization precedes replay, ADR-0031 item 9).
4. **Given** a list for a name no published version declares, **When** a later version declares it,
   **Then** the list applies from the first call.
5. **Given** a developer with `definition:write` but not `role:assign`, **When** they add themselves to the
   list or delete it, **Then** `PRIVILEGE_ESCALATION`; removing a principal or creating a list succeeds.

### US06 — See and explain effective access (Priority: P2)

A member sees their permissions in a namespace; an administrator asks why someone may or may not cancel a
run and receives the decision and its sources.

**Why this priority**: an access model nobody can explain is not operable.

**Independent Test**: compare explanations with the evaluator over every test vector; the Studio shows
the same.

**Acceptance Scenarios**:

1. **Given** any member, **When** they ask for their auth context in a namespace, **Then** they get their
   effective namespace permissions and their levels on folders they can see.
2. **Given** an `access:read` holder, **When** they explain (subject, permission, target), **Then** they get
   the decision and its sources (built-in role, role, revision, assignment, group, folder level, list).
3. **Given** a target or folder the explainer cannot see, **When** asked, **Then** `404` as for a missing one.

### US07 — Put scripts and database connections in folders (Priority: P3)

The folder contract applies to scripts (test runs, executions) and to connections; a connection in a
restricted folder is usable by a published workflow only if its publisher or initiator is listed at
`runner` or above.

**Why this priority**: workflows are the main need; these follow once their store rechecks use the
evaluator.

**Independent Test**: the US03 battery over script and connection operations; admit runs of a dependent
workflow as listed and unlisted publisher; write a connection as a non-administrator.

**Acceptance Scenarios**:

1. **Given** a connection in a restricted folder and an unlisted publisher, **When** a dependent run is
   admitted, **Then** it is refused as for a revoked 017 permission (`DATABASE_UNAVAILABLE` for a child,
   `database.denied` for a triggered start).
2. **Given** a script in a restricted folder, **When** an unlisted author publishes a workflow pinning it,
   **Then** publication fails as for a missing script.
3. **Given** a non-administrator holding `database:manage` through a custom role, **When** they write a
   connection that selects a secret they may not use, **Then** it is refused as for a missing secret.

### US08 — Identity-provider groups stay out until a source exists (Priority: P3)

Until an approved source feature implements contracts/group-source.md (O1440), provider groups grant
nothing and cannot be created here.

**Why this priority**: prevents a premature or claim-based group path that could not be revoked in time.

**Independent Test**: present tokens carrying group and role claims; try to create an external group.

**Acceptance Scenarios**:

1. **Given** a group or role claim inside an access token, **When** the token is used, **Then** it confers
   nothing.
2. **Given** a request to create a group with an external source, **When** it is sent, **Then** `422`
   `ACCESS_CAPABILITY_DISABLED`.

### Edge Cases

- 32 groups with overlapping roles at both scopes, inputs permuted; a role edited between a request's
  authorization and commit; a run stream open while its folder becomes hidden.
- An item moved into a restricted folder while a run waits for an approval whose deciders are unlisted
  (it reaches its deadline; nobody unlisted can decide); a visible parent whose child becomes hidden
  (the parent's journal still carries the child's outcome, G1456).
- A level-only publisher publishing a workflow with event or cron triggers (needs `event:manage` in the
  ceiling, FR-029-023).
- A membership deleted between preview and apply; group or assignment changes after a preview (advisory,
  G1458); a namespace role whose namespace is deleted.
- The last owner of a folder leaves (`folder:manage` holders still administer it).
- A folder renamed while a manifest names it; promotion into an existing hidden or open target.
- Every ceiling reached; quota lowered below usage; cross-tenant identifiers in every field; machine
  identities as principals; crash between change and audit or preview and apply.
- A replica running a binary older than this feature while restriction is enabled (FR-029-037).

## Requirements *(mandatory)*

### Functional Requirements

**Catalog and built-in roles**

- **FR-029-001**: This feature MUST NOT remove or narrow any built-in role or grant, MUST keep one built-in
  role per membership, and authentication MUST still establish tenant, subject and built-in role from the
  membership record. Built-in roles are additive: roles and grants that other features add are honoured as
  they are, machine roles they add are never principals, and this feature's tests assert set properties
  (`admin` holds every assignable classified permission; the five new permissions go to `admin` only; every
  level permission is held by `developer`; human roles are exactly admin, developer, operator, viewer), not
  counts. Outside restricted folders and operation lists, the built-in role is the floor of access; custom
  grants only add.
- **FR-029-002**: The catalog MUST gain `access:read`, `role:manage`, `role:assign`, `group:manage` and
  `folder:manage`, granted to built-in `admin` only. One classification in the domain (a compile-time default of reserved), shared by
  server, contract and Studio, MUST give each permission its assignability, scopes and item class and kinds
  (contracts/permission-catalog.json); a permission added later MUST be reserved until classified.
- **FR-029-003**: `worker:execute`, `script:build`, `script:execute`, `database:execute`,
  `payload:read-internal`, `payload:reveal-pii`, `membership:manage`, `group:manage`, `worker:bind` and
  `secret:manage` are reserved and MUST NOT be granted by a custom role, folder level or operation list
  (`RESERVED_PERMISSION`), so the ADR-0033 audiences stay as they are and no custom grant can route secrets
  or hidden inputs to a worker or rewrite secret lists. Other built-in grants are defaults, not ceilings
  (ADR-0064 lists the decisions this refines); `quota:manage`, `egress:manage` and `audit:read` are marked
  administrator-equivalent and granting them MUST require an explicit confirmation field in the request
  and the Studio; `audit:read` is marked as seeing past restriction.

**Custom roles and assignments**

- **FR-029-004**: A `role:manage` holder MUST be able to create, edit and delete custom roles: a slug name
  unique within its owner (the tenant's roles, or one namespace's roles), a description of at most 512
  bytes, an owner (tenant or one namespace) and a non-empty set of assignable permissions meaningful in that
  owner's scope (`PERMISSION_SCOPE_MISMATCH`). Edits use the role revision for optimistic concurrency,
  create an immutable revision and apply to every assignee on their next admitted call. A role with
  assignments MUST NOT be deleted (`ROLE_IN_USE`). The scope of an existing role or assignment MUST come
  from its stored row, never from a caller-supplied namespace.
- **FR-029-005**: A `role:assign` holder MUST be able to assign and unassign a role to a human member or a
  local group, tenant-wide or in one namespace; a namespace role only in its namespace. Assigning is
  idempotent per (role, principal, scope) and records granter and time. An ineligible principal (unknown
  subject, machine identity) MUST be refused with `UNKNOWN_SUBJECT` or `MACHINE_IDENTITY` for an
  `access:read` holder and with `PRINCIPAL_NOT_ELIGIBLE` for anyone else (no member enumeration).
- **FR-029-006** *(non-escalation)*: A grant MUST be refused with `PRIVILEGE_ESCALATION` unless the actor,
  evaluated on committed state inside the writing transaction, administers its scope and holds every
  permission it confers wherever it applies: assignments and role edits in their scope; folder entries on
  the folder's items; **moves**, whose destination entries start applying to the item; **widening an
  operation list**; and **adding a member to a group**, which grants everything that names the group
  (contracts/evaluation.md, "Administration rules").
- **FR-029-007**: An assignment MUST NOT depend on its granter after commit, and `access:read` holders MUST
  be able to list assignments and entries by role, principal, scope and granter.

**Groups**

- **FR-029-008**: A `group:manage` holder MUST be able to create, rename, describe and delete local groups
  and add and remove human members; groups MUST NOT nest; a referenced group MUST NOT be deleted
  (`GROUP_IN_USE`); deleting a membership MUST remove the subject from every group in the same transaction.
  Adding a member MUST satisfy `canAddMember` and MUST be audited as a widening of every operation list and
  restricted-folder entry that names the group.
- **FR-029-009**: Identity-provider groups MUST NOT be implemented by this feature. Token group or role
  claims MUST NOT confer any permission, and creating a group with an external source MUST be refused
  (`ACCESS_CAPABILITY_DISABLED`). A future source MUST meet contracts/group-source.md (O1440).

**Folders and items**

- **FR-029-010**: A `folder:manage` holder in a namespace MUST be able to create, rename and delete folders
  (namespace-unique slug, display name ≤ 128 bytes, mode `open` by default or `restricted`) and change any
  folder's mode and entries. Folders MUST NOT nest. A folder containing a live item MUST NOT be deleted
  (`FOLDER_NOT_EMPTY`); deleting an empty one removes its entries and the placements of tombstoned items in
  the same transaction. A folder the caller cannot see (contracts/evaluation.md, `visibleFolder`) MUST be
  answered and listed as missing, including when named by a create, move or manifest.
- **FR-029-011**: Workflows, scripts and database connections MUST each have at most one placement. Names
  MUST stay unique per namespace and kind regardless of folder. Creating an item in a folder requires the
  create permission there; at the root it is decided as today.
- **FR-029-012**: A move MUST satisfy `canPlace`: the item's write permission, administration of source and
  destination (root: `folder:manage`), `folder:manage` when the source is restricted, and every item
  permission the destination's entries confer held by the mover on the item. It MUST commit placement, both
  folder revisions and audit together without changing identity, name, versions, runs or references.
- **FR-029-013**: Levels MUST confer exactly (cumulatively): `viewer` = `definition:read`, `script:read`,
  `run:read`, `approval:read`, connection visibility; `runner` = + `run:start`, `run:query`, `run:update`,
  `database:use`; `editor` = + `definition:write`, `definition:publish`, `script:write`, `script:publish`;
  `owner` = `editor` + administration of that folder's entries. Every level permission MUST be one the
  built-in `developer` role holds, and applies only to its item kinds. All other item permissions are
  sensitive and never conferred by a level. "Bounded by developer" limits who may act, not where effects
  land: an `editor` may publish workflows whose activities act across the namespace (workers bound to those
  activities, connections it may use); secrets and triggers still need namespace permissions (FR-029-023).
- **FR-029-014**: An entry MUST name a human subject or local group, one level, granter and time. A folder
  owner or `folder:manage` holder MAY set and remove entries subject to FR-029-006, the actor's power on the
  folder being their namespace ceiling plus their own level there (so a `folder:manage` holder can always
  administer). Granting `owner` requires administering the folder. Changing the mode requires
  `folder:manage`.
- **FR-029-015** *(item evaluation)*: At the root or in an open folder, a level permission MUST be allowed
  when in the namespace ceiling or the subject's levels on that folder; in a restricted folder, only when in
  those levels. A sensitive item permission MUST require the namespace ceiling and visibility of the item.
  Built-in `admin` MUST NOT bypass a restricted folder except through an audited change of its entries.
- **FR-029-016**: Before a change that removes visibility (restricting, removing or lowering an entry,
  moving items into a restricted folder) the platform MUST offer a bounded, value-free preview: subjects
  and groups losing visibility, pending approvals whose deciders lose it, affected non-terminal runs and
  published parents referencing items of the folder. It MUST count and identify only items visible to the
  previewer and report the rest as one opaque total, and only to `folder:manage` or `access:read` holders.
  Applying it MUST carry the preview's folder revision (`REVISION_CONFLICT` if it moved); principal
  identifiers are returned only to `access:read` holders.
- **FR-029-017** *(opacity)*: An item or folder operation MUST first refuse with `403` before any lookup
  when the caller could not hold the permission on any item of that kind (neither ceiling nor any of their
  folder levels), as today; only then is the target resolved, by one statement for item and placement, and
  missing and hidden targets MUST answer the same `404`. For a caller who cannot see an item, every operation on it or anything
  belonging to it (version, draft, publication request, run, chain, journal, graph, stream, telemetry,
  attempts, approval, payload, declared operation, database result, execution) MUST answer exactly as for
  a missing resource and listings MUST omit it. In addition: a name-keyed write or lookup (create, upsert,
  promotion target, manifest publish) MUST resolve visibility before any other branch, never read or write a
  hidden target, and answer as the operation's ordinary name collision (`NAME_UNAVAILABLE` where it had
  none); an idempotent replay MUST NOT return a stored response that names a hidden item
  (`IDEMPOTENCY_KEY_REUSED`); rate admission MUST charge a hidden target exactly as a missing one; a probe
  of a hidden item MUST be audited off the response path. Aggregates and quotas MAY count hidden items
  (G1442); timing is not equalized and the measured difference is published (G1457); audit reads are not
  filtered (G1451).
- **FR-029-018**: A run MUST follow its workflow's current folder; a child run its own workflow's. Private
  017 ad hoc and probe definitions MUST keep their actor-scoped access and stay out of folders.

**Operation lists**

- **FR-029-019**: A holder of `definition:write` on a workflow MUST be able to create a list for (workflow,
  `query`|`update`, name) and remove principals from it; adding principals or deleting the list widens
  access and MUST also require `role:assign` in the namespace and the right to call that operation that the
  actor would have without the list (`run:read` and `run:query` or `run:update` on the workflow's runs), so
  an access administrator can always widen a list, audited, and no list can lock everyone out. While a list exists, every call to that name (run- or chain-addressed, including receipt
  replay) MUST also require the caller or one of their groups to be listed; otherwise ADR-0031 applies
  unchanged. Lists are keyed by name across versions, may precede a declaration, and never widen beyond
  `run:read`/`run:query` or `run:update`; a refusal is `403` `OPERATION_NOT_GRANTED`, writes no receipt and
  is audited.

**Evaluation, revocation and coverage**

- **FR-029-020** *(determinism)*: Every decision MUST be computed by one pure function of the built-in
  role, groups, assignments at tenant scope and in the namespace, the roles they name, the target's
  placement, folder mode and entries, the operation list, the permission and the target; independent of
  input order; deciding in the order of FR-029-017 (permission before lookup, then visibility); with no
  narrowing other than FR-029-015 and FR-029-019; never yielding a reserved permission from a custom grant;
  returning its contributing sources. Listings MUST use the same function's `visibleFolders(kind)` result
  as a query parameter; no SQL predicate re-derives the rules.
- **FR-029-021** *(revocation)*: Decisions MUST NOT be cached across requests unless the cache is versioned
  by every access change. A committed change MUST govern every request authorized after it. Every
  store-side recheck of a sensitive or item permission (inventoried in plan.md, including declared
  Query/Update, approval decisions, payload disclosure and reveal, event payload reads, saga overrides,
  publication approvals, database admission, ad hoc, write, use and results, managed secrets, secret use at
  publication and resolution) MUST use the evaluator behind the access fence. A long-lived stream MUST
  re-evaluate on resume and at least every 30 seconds and end without disclosure when access is gone.
- **FR-029-022** *(coverage)*: Every REST operation MUST declare in the OpenAPI document its decision and
  the item kinds its answers can disclose (`x-access`). A contract test MUST fail on a missing declaration
  or on a response schema carrying item identifiers without `discloses`. Before restriction is enabled for
  an item kind, an opacity battery MUST attack every operation that names or discloses that kind as a hidden
  caller of every built-in human role and as a level-only member, and a canary scan MUST find no hidden
  canary item's name or identifier in any response, header or rate-limit effect, audit reads excepted
  (G1451). Restricted mode and operation lists MUST stay disabled per kind
  (`ACCESS_CAPABILITY_DISABLED`) until both pass, because a route checking only the built-in role would
  fail open.
- **FR-029-023**: Engine-internal admissions that act for no recorded subject (event and cron triggers
  declared in a published document, child starts, scheduler internals) MUST NOT be re-authorized by
  folders. Actions taken on behalf of a recorded subject MUST be rechecked through the evaluator at fire or
  submit time: an 018 schedule action against its `authorizedBy` subject (FR-018-015: the subject must still
  see the target workflow and hold `run:start` on it, else `AUTHORITY_REVOKED` and pause) and a 026 public or
  decision link submission against its owner (else the link refuses). Connections in folders are rechecked
  at admission and capability use, as 017 does today. Publication MUST require `run:start` on every
  referenced child workflow, `script:read` on every pinned script and `database:use` on every connection,
  answering as for a missing reference when the target is hidden; publishing a version that declares
  triggers with a publish right that comes only from a folder level MUST also require `event:manage` in the
  namespace ceiling.
**Lifecycle, surfaces and contracts**

- **FR-029-024**: Changing a membership's built-in role MUST remove, in the same transaction and audited,
  the subject's direct assignments, group memberships, folder entries and list entries (as 016 D663 does
  for reveal grants; O1441).
- **FR-029-025**: A manifest entry MAY name a folder (additive); manifest placement MUST go through the move
  rules and MUST NOT create folders or change entries. Promotion MUST resolve the destination target's
  visibility first (FR-029-017); promoting a workflow from a restricted folder MUST name a destination
  folder (`DESTINATION_FOLDER_REQUIRED`) and, for the root or an open folder, requires `folder:manage` in
  the destination namespace.
- **FR-029-026**: `GET /auth/context` MUST accept an optional namespace and return the caller's effective
  permissions there (additive). An `access:read` holder MUST be able to explain a decision for any member;
  any member MAY explain their own.
- **FR-029-027** *(audit)*: Every access change MUST be audited in its transaction with a server-built
  reason and no free text; an audit failure MUST roll the change back. Refused grants and probes of hidden
  items MUST be audited on the collapsing best-effort denial path, which never authorizes on failure.
- **FR-029-028** *(errors)*: Errors MUST reuse the closed `Error.code` set, adding only `details.reason`
  values with remediation rows (contracts/README.md); a hidden item carries no reason.
- **FR-029-029** *(observability)*: Metrics MUST use closed labels only (no tenant, subject, role, group,
  folder or item identifier); span attributes come from closed sets; logs are redacted; names and
  descriptions never reach metrics or logs.
- **FR-029-030** *(ceilings)*: Custom roles and groups per tenant MUST be tenant quota resources (effective =
  minimum of request, tenant setting and operator ceiling) and folders per namespace a namespace resource
  (minimum of request, namespace setting, tenant default and ceiling); zero refuses creation. Namespace roles
  per namespace and every per-entity bound of the plan MUST be enforced before work (`ACCESS_LIMIT`). A
  lowered limit MUST NOT delete rows; removal stays possible.
- **FR-029-031** *(Studio)*: The Studio MUST offer Access screens (roles, assignments, groups, folders and
  entries, operation lists, explanation), a folder tree and move action, the restriction preview (warning
  that item names are not confidential, G1441), an explicit confirmation before granting an
  administrator-equivalent permission, and hide actions the server's effective permissions disallow without
  deciding access itself; WCAG 2.2 AA, design tokens only, no sample values where there is no data
  (FR-007-010).
- **FR-029-032** *(CLI)*: `sagat access` MUST cover roles, assignments, groups, folders, entries, moves,
  operation lists and explanation with REST semantics and errors.
- **FR-029-033** *(SDKs)*: The five SDKs MUST surface the new reasons without new error codes and pass
  shared scenarios for a custom grant, a hidden item, an operation-list refusal and revocation; the worker
  protocol and IR MUST NOT change.
- **FR-029-034** *(compatibility)*: Migrations, OpenAPI and manifest changes MUST be additive; for a tenant
  with no access objects every response MUST be unchanged except the additive `accessControl` capabilities
  of `GET /version` (`folder` is omitted when absent); down migrations MUST refuse while access rows exist.
- **FR-029-035** *(tenant boundary)*: Every row MUST carry its tenant (and namespace where scoped) in keys
  and composite foreign keys; every identifier MUST resolve inside the authenticated tenant; another
  tenant's subject, group, role, folder or item MUST be indistinguishable from a missing one, with the probe
  audited.

**Credential selection and binary compatibility**

- **FR-029-036**: Writing a database connection by anyone other than built-in `admin` MUST require
  `secret:use` in the ceiling and the selected secret's access list, answering as for a missing secret
  otherwise (017 lets the catalog writer select any secret; only `admin` held `database:manage` until now).
- **FR-029-037** *(binary guard)*: Restricted mode and operation lists MUST be usable only when every
  serving binary honours them, through feature 018's instance-capability registry (018 data-model,
  "Instance-capability registry"; T018-080), deployed fleet-wide in a release before this feature's
  narrowing: every control-plane process registers before serving and heartbeats, and re-checks the
  requirements on every API admission and stream re-evaluation (`503` `BINARY_CAPABILITY_MISSING` while it
  lacks a recorded capability). Activating `access-narrowing-v1` (kind `CONTROL_PLANE`) MUST be an audited
  operator act of a deployment administrator through 018's activation command, refused while any live
  control-plane instance lacks it; no tenant transaction activates it. Until it is recorded, creating a
  restricted folder, changing a folder to `restricted` or creating an operation list MUST be refused
  (`ACCESS_CAPABILITY_DISABLED`). Removing the requirement MUST be possible only when no restricted folder
  and no list remains.

### Non-functional requirements

- **NFR-029-SEC**: Zero tested grants (including moves and list widening) beyond the granter's power,
  reserved permissions reached through custom grants, hidden items or folders disclosed by any operation,
  or cross-tenant references accepted; each blocks release. Authorization fails closed on every store or
  evaluation error.
- **NFR-029-DUR**: An access change and its audit commit atomically; a sensitive decision and a concurrent
  change serialize through the fence; a crash leaves both or neither. Run transitions, scheduling, timers,
  subscriptions and the outbox are untouched.
- **NFR-029-COMP**: Existing clients, SDKs, Studio builds, manifests and runs keep working; additive REST
  only; unchanged responses for tenants without access objects (FR-029-034).
- **NFR-029-OBS**: Every change audited; outcomes, changes and fence waits counted with closed labels;
  explanations reproducible from durable rows.
- **NFR-029-PERF**: Added p50/p95/p99 latency per authorized request, per filtered listing (1,000 folders,
  64 entries each) and fence waits under a long sensitive reader, measured on a declared environment and
  published; no promise before that.
- **NFR-029-ACC**: Access screens meet WCAG 2.2 AA (keyboard, labels, levels not by colour alone), checked
  by tools and a keyboard walkthrough.

### Key Entities

- **Permission catalog entry**: permission, assignability, scopes, item class and kinds (closed table; unknown permissions default to reserved).
- **Custom role / revision**: owner (tenant or namespace), name, description, permission set; immutable
  revisions.
- **Assignment**: role, principal (subject or local group), scope, granter, time.
- **Group / member**: local, name, description, human members.
- **Folder / entry / placement**: namespace, name, mode, revision; entries (principal, level); at most one
  placement per workflow, script or connection.
- **Operation list**: workflow, kind, name, listed principals.
- **Capability requirement**: `access-narrowing-v1` in feature 018's instance-capability registry (not a 029
  table).
- **Access decision**: outcome with contributing sources; never stored except in audit.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-029-001**: For every acceptance scenario of US01–US08, an automated suite on real PostgreSQL and
  HTTP passes as the runtime database role; each refusal leaves no row and one bounded audit. Traces:
  FR-029-001, FR-029-004, FR-029-005, FR-029-007–FR-029-012, FR-029-014, FR-029-016, FR-029-018,
  FR-029-019, FR-029-023, FR-029-024, FR-029-028, FR-029-036.
- **SC-029-002**: The shared vectors (at least one per rule of FR-029-013/015/019/020) and 10,000 generated
  cases give identical decisions under every input permutation, never a reserved permission from a custom
  grant, never a level permission outside the developer role or its kinds; the catalog fixture equals the
  server table for every permission this feature classifies, every other permission is reported reserved,
  and the built-in role set properties of FR-029-001 hold whatever roles and grants other features add;
  explanations equal the evaluator for every vector. Traces: FR-029-001, FR-029-002, FR-029-003,
  FR-029-013, FR-029-015, FR-029-019, FR-029-020, FR-029-026.
- **SC-029-003**: The escalation matrix (roles, assignments, entries, moves, list widening) has zero accepted
  grant beyond the granter's power and zero refused grant within it. Traces: FR-029-006, FR-029-012,
  FR-029-014, FR-029-019.
- **SC-029-004**: For each enabled item kind, the opacity battery covers 100% of operations that name or
  disclose it as every built-in human role and a level-only member, each hidden-caller answer equals the
  missing-target answer byte for byte (except trace and request ids) and in rate-limit effect, listings omit hidden items and folders, name-keyed writes and key
  replays never touch or return a hidden item, the canary scan finds nothing, and the cross-tenant battery
  covers every new operation. Traces: FR-029-010, FR-029-017, FR-029-018, FR-029-023, FR-029-035.
- **SC-029-005**: `x-access` is present on 100% of operations and `discloses` on every response that can
  carry item identifiers; restricted mode and lists are refused for a kind while its battery or scan is
  incomplete. Traces: FR-029-022.
- **SC-029-006**: After each kind of change commits, the next admitted request of an affected subject
  follows the new state in 100% of trials; under injected concurrency every sensitive decision sees the old
  or the new state as a whole. Traces: FR-029-006, FR-029-021.
- **SC-029-007**: A crash at each statement of every access change, preview/apply and membership deletion
  leaves the change with its audit or neither; duplicates have no second effect. Traces: FR-029-016,
  FR-029-024, FR-029-027.
- **SC-029-008**: A run stream of an item that becomes hidden to its caller ends within 30 seconds in 100%
  of trials and discloses nothing afterwards. Traces: FR-029-021.
- **SC-029-009**: All five SDKs pass the shared access scenarios with identical normalized outcomes and no
  new error code. Traces: FR-029-028, FR-029-033.
- **SC-029-010**: A real-browser journey creates a role, a group and a restricted folder with preview,
  moves a workflow and shows an explanation; automated accessibility checks report no AA violation. Traces:
  FR-029-031.
- **SC-029-011**: Each ceiling is reached in a test that ends `QUOTA_EXCEEDED` before any write; a lowered
  quota keeps existing rows usable and removable. Traces: FR-029-030.
- **SC-029-012**: Captured logs, spans, metrics, audit and error bodies contain no description, display name
  or planted canary, and every label value is in its closed set. Traces: FR-029-027, FR-029-029.
- **SC-029-013**: The benchmark report states its environment and the measured added latency percentiles and
  fence waits at the plan's ceilings; nothing is claimed beyond it. Traces: NFR-029-PERF.
- **SC-029-014**: An upgraded tenant without access objects passes the existing contract, security and SDK
  suites with unchanged responses apart from the declared carve-out. Traces: FR-029-001, FR-029-034.
- **SC-029-015**: Promotion and manifest placement obey FR-029-025 in 100% of trials; manifests without the
  field behave as before. Traces: FR-029-025.
- **SC-029-016**: The quickstart's CLI walkthrough (happy and failure paths) runs end to end on the real
  stack and each CLI answer equals the REST answer. Traces: FR-029-032.
- **SC-029-017**: Before the operator activates `access-narrowing-v1`, every attempt to create a restricted
  folder or an operation list is refused; with a live instance lacking the capability, the operator's
  activation is refused in 100% of trials; after activation, such a binary answers `503` on its next
  admission (including after a pause longer than the liveness window); the requirement cannot be removed
  while a restricted folder or list exists. Traces: FR-029-037.

## Assumptions

- Human identities authenticate as today (001; 013 draft); machine identities keep built-in roles and
  bindings and are never principals here.
- Members exist (operator provisioning or test support, G290).
- Access changes are rare compared with authorization reads; serializing access writes per tenant is
  acceptable, to be confirmed by the benchmark.
- Folder levels are fixed in this increment.
- "Resources" in the input means 017 connections today and 021 resources when that feature adopts the
  contract; "apps" means 027 apps under the same condition.
- Upgrades pass through the release that ships 018's instance-capability registry before restriction is
  enabled (018 G1607).

## Dependencies

- Existing: 001, 002, 003 (`saga:override`), 005, 007, 008, 009 (bindings, lists, quotas, cross-tenant
  battery), 011 (manifests, promotion), 014 (declared operations, approvals, chains), 016 (reveal grants,
  D663), 017 (connections, admission rechecks, runtime-role lock functions); ADR-0001, 0002, 0014, 0017,
  0018, 0027, 0028, 0031, 0033, 0035, 0036.
- New features: 013 (group source, O1440); 021 (folder placement for resources) and 027 adopt the folder
  contract; when 029 is enabled, visibility is decided by the 029 evaluator for 018 batch selection and
  schedule actions (rechecked at fire time), 020 list queries and saved views, 021 resources, 025 MCP tool
  listing and calls (which also honour operation lists) and 026 public and decision links (rechecked at
  submit time); 030 authorizes callers by namespace, not as folder principals; 018, 019, 020, 021, 022, 024, 025, 026, 027, 028, 030, 031, 032 add permissions
  (and 021, 025, possibly 020, add machine roles) that this catalog classifies, reserved until then;
  019's new routes (task completions that return a run status, repin on a run, worker insights) need
  `x-access` and `discloses` declarations, and its `push:manage` and `task:complete` should be at least
  sensitive.
- Lock order: [ADR-0050](../../docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md)
  decision 7 (018): the 029 access-capability check is a read in slot 1 (with the 026/027 rows);
  memberships, the 029 access fences and the namespace row are slot 4a, after slots 2, 3 and 3R.
- Proposed decision: [ADR-0064](../../docs/adr/ADR-0064-access-grants-add-and-only-restricted-folders-and-operation-lists-narrow.md).

## Out of Scope

Deny rules and policy languages; custom folder levels; nested folders or groups; delegated group
administration (G1445, G1446); member management (G290, O1442); identity-provider groups (013, O1440);
approval eligibility by group or custom role (G1448); secrets in folders (G1450); folder filtering of audit
reads or worker bindings; per-run or per-version access lists; any change to ADR-0033 audiences.

## Owner questions

- **O1440 — Who delivers identity-provider groups?** Recommended: amend 013 (or a successor) to push SCIM
  2.0 Groups into tenant groups under contracts/group-source.md; token claims never authorize. Until then,
  local groups only.
- **O1441 — Does a built-in role change revoke custom grants?** Recommended: yes (FR-029-024), as D663 for
  reveal grants.
- **O1442 — Should this feature add member management (G290)?** Recommended: no; it stays with 013 or a 009
  follow-up.

See [research](research.md), [plan](plan.md), [data model](data-model.md), [contracts](contracts/README.md),
[quickstart](quickstart.md), [tasks](tasks.md) and the [requirements checklist](checklists/requirements.md).
