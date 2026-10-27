# Contracts — feature 029 (draft fragments)

Draft only. Nothing here is in the canonical contracts yet; task T029-066 merges these fragments into
`specs/001-platform-foundation/contracts/openapi.yaml`, `specs/011-gitops-versioning/contracts/manifest.schema.json`,
the remediation table and the conformance suite at implementation time.

| File | What it defines |
|---|---|
| [access.openapi.yaml](./access.openapi.yaml) | New REST paths and schemas, additive fields on existing schemas |
| [permission-catalog.json](./permission-catalog.json) | The classification of all 53 permissions and the folder levels |
| [evaluation.md](./evaluation.md) | The normative evaluation function, administration rules and test vectors |
| [group-source.md](./group-source.md) | The contract a future identity-provider group source must satisfy (not built here) |
| [conformance.md](./conformance.md) | Five-SDK scenario, opacity battery, canary scan, escalation matrix, coverage test |

Unchanged on purpose (D1461): the workflow IR (`workflow.schema.json`), the worker protocol
(`worker.proto`, no minor bump), the AsyncAPI document (no event is published), `Error.code`.

## The `x-access` operation extension

Every operation of the canonical OpenAPI document MUST carry `x-access`. Clients ignore it. The route
registry is generated from each handler's typed access extractor and compared with it
(`contract_rest_custom_roles_folders::coverage`).

```yaml
x-access:
  class: item              # public | authenticated | tenant | namespace | folder | item | machine | endpoint
  permission: run:read     # all required (string or list); or `anyOf: [...]` when any one suffices
  kind: run                # item only: workflow | run | script | connection
  target: runId            # item or folder: the path, header or body field that names it
  discloses: [workflow]    # item kinds (or `folder`) whose identifiers, names or data an answer can contain
  widenRequires: [role:assign]   # operation lists only: extra permissions when the write widens access
```

- `public`: health and readiness. `authenticated`: auth context, permission catalog. `endpoint`: webhook
  ingress authenticated by its profile (ADR-0026). `machine`: worker and script-runner routes decided by
  machine role, binding and lease (unchanged, D1459). `tenant`/`namespace`: decided by `tenantWide` and
  `ceiling`. `folder`: decided by `visibleFolder` plus the permission. `item`: decided in the order of
  evaluation.md (`403` before any lookup when the caller could not hold the permission on that kind, then
  missing and hidden alike `404`, then `allowItem`).
- `anyOf` pseudo-grants: `self` (the subject named by the request is the caller), `folder-owner` (owner level
  on the folder named by `target`).
- For an existing role, assignment, entry or list, the scope used for the decision is the one stored with
  the object, never a caller-supplied `namespaceId`. For a create, it is the scope in the body.
- `discloses` is mandatory on every operation whose response schema can carry `workflowId`, `runId`,
  `scriptId`, a connection name, a folder name or item data (for example runs, chains, approvals, alerts,
  event inbox and dead letters, publication requests, script executions). The opacity battery and the canary
  scan read it.

## Capabilities

`GET /version` gains (additive; the only response change for a tenant without access objects):

```yaml
accessControl:
  customRoles: true
  groups: true
  folders: true                      # open folders and placement
  restrictedFolders: [workflow, run] # item kinds whose battery, scan and requirement check passed
  operationLists: false              # same gate, for runs
  externalGroups: false              # not built here (O1440)
```

A request that needs a disabled capability is `422 VALIDATION_FAILED` with `ACCESS_CAPABILITY_DISABLED`.
`restrictedFolders` and `operationLists` stay empty or `false` until a deployment administrator has
activated `access-narrowing-v1` through feature 018's instance-capability registry (an audited operator
command; activation is never a side effect of a tenant request).

## Errors

The closed `Error.code` set is unchanged; these `details.reason` values are added, each with a row in the
remediation table of `crates/api-http/src/error.rs`.

| `details.reason` | `code` | HTTP | When |
|---|---|---|---|
| `RESERVED_PERMISSION` | `VALIDATION_FAILED` | 422 | A role lists a reserved permission |
| `UNKNOWN_PERMISSION` | `VALIDATION_FAILED` | 422 | A role lists a name not in the catalog |
| `PERMISSION_SCOPE_MISMATCH` | `VALIDATION_FAILED` | 422 | A namespace role lists a tenant-only permission |
| `MACHINE_IDENTITY` | `VALIDATION_FAILED` | 422 | A principal is a machine identity (caller holds `access:read`) |
| `UNKNOWN_SUBJECT` (exists, 016) | `VALIDATION_FAILED` | 422 | No membership in the tenant (caller holds `access:read`) |
| `PRINCIPAL_NOT_ELIGIBLE` | `VALIDATION_FAILED` | 422 | Either of the two above, for a caller without `access:read` |
| `DESTINATION_FOLDER_REQUIRED` | `VALIDATION_FAILED` | 422 | Promotion of a workflow from a restricted folder names no destination |
| `ACCESS_CAPABILITY_DISABLED` | `VALIDATION_FAILED` | 422 | The capability or item kind the request needs is not enabled |
| `PRIVILEGE_ESCALATION` | `FORBIDDEN` | 403 | A grant, move or list widening exceeds the actor's power |
| `OPERATION_NOT_GRANTED` | `FORBIDDEN` | 403 | The caller is not on the operation list of that name |
| `ROLE_IN_USE` | `CONFLICT` | 409 | Deleting a role that has assignments |
| `GROUP_IN_USE` | `CONFLICT` | 409 | Deleting a referenced group |
| `FOLDER_NOT_EMPTY` | `CONFLICT` | 409 | Deleting a folder with a live item |
| `NAME_UNAVAILABLE` | `CONFLICT` | 409 | A name-keyed write meets a hidden item and the operation has no ordinary collision answer |
| `REVISION_CONFLICT` (exists) | `CONFLICT` | 409 | Stale role or folder revision, including a stale preview |
| `IDEMPOTENCY_KEY_REUSED` (exists) | `CONFLICT` | 409 | Also: a replay whose stored response names an item now hidden |
| `ACCESS_LIMIT` | `QUOTA_EXCEEDED` | 429 | A per-entity ceiling of the plan is reached |
| `ACCESS_WRITE_CONTENDED` | `UNAVAILABLE` (retryable) | 503 | An access writer could not take its fence or `NOWAIT` lock after bounded retries |
| `BINARY_CAPABILITY_MISSING` (defined by 018's instance-capability registry) | `UNAVAILABLE` (retryable) | 503 | This instance lacks a capability the database requires (another instance can serve) |
| `CONFIRMATION_REQUIRED` | `VALIDATION_FAILED` | 422 | Granting an administrator-equivalent permission without the explicit confirmation field |

A caller who cannot see an item or folder receives `404 NOT_FOUND` with no reason. A name collision with a
hidden item uses the operation's ordinary collision answer when it has one (for example
`createWorkflowDraft` answers `409 Conflict` as today). Count quotas answer with the existing
`QUOTA_EXCEEDED` body of feature 009.

## Audit actions and resource types

Actions (server-built reasons, no free text): `role.created`, `role.changed`, `role.deleted`,
`role.assigned`, `role.unassigned`, `group.created`, `group.changed`, `group.deleted`,
`group.member_added`, `group.member_removed`, `folder.created`, `folder.changed`, `folder.deleted`,
`folder.entry_set`, `folder.entry_removed`, `folder.entry_widened` (a member added to a group named by an
entry of a restricted folder), `item.placed` (create in a folder, move, manifest placement),
`operation_list.created`, `operation_list.narrowed`, `operation_list.widened` (also for a member added to a
listed group, reason `via=group:<uuid>;member=subject:<subject>`), `operation_list.removed`,
`access.revoked_on_role_change`, `access.denied.escalation`,
`access.denied.hidden`. Reused: `access.denied.permission`, `access.denied.cross_tenant`.

Resource types: `custom_role`, `role_assignment`, `access_group`, `folder`, `operation_list`; placement uses
the item's existing type (`workflow`, `script`, `database_connection`).

Reason grammar examples: `role.changed` → `rev=4;permissions=definition:read,run:operate,version:activate`;
`role.assigned` → `role=<uuid>;principal=subject:<subject>;scope=namespace:<uuid>`;
`folder.entry_set` → `principal=group:<uuid>;level=runner`; `item.placed` →
`from=<folder-uuid|root>;to=<folder-uuid|root>`. Every field comes from a closed vocabulary, a UUID or a
subject already bounded by the membership grammar; names, display names and descriptions never appear.

## GitOps manifest (additive)

Manifest entries gain an optional `folder` (slug). `sagat publish` resolves the folder's visibility and
applies the move rules (`canPlace`); it never creates a folder or changes entries. A hidden target name
answers as a collision. An older CLI rejects a manifest that uses the field (`additionalProperties: false`).
