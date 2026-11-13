# Access evaluation — normative definition and test vectors (draft)

The domain function `sagat_domain::access::evaluate` implements this page; the vectors become the fixture
`tests/contract/custom-roles-folders/fixtures/evaluation-vectors.json` (T029-004), plus generated cases
(SC-029-002). Notation: `B` built-in role of the membership; `perms(x)` a permission set; `∪` union.

## Inputs (`AccessFacts`)

| Fact | Source | Notes |
|---|---|---|
| `builtin` | `memberships.role` | machine roles produce facts with no custom part; human roles are exactly `admin`, `developer`, `operator`, `viewer`, and any role another feature adds is a machine role |
| `groups` | `access_group_members` of the subject (local groups only in this feature) | ≤ 64 |
| `tenantAssignments` | assignments of the subject or its groups with tenant scope | current role revision |
| `namespaceAssignments(N)` | assignments in namespace N | idem |
| `folder(i)`, `mode` | `item_placements` of the item; the folder's `mode` | `root` when no placement |
| `entries(F)` | entries naming the subject or one of its groups | maximum level per principal |
| `operationList(w,k,n)` | `operation_lists` and principals | absent, or a set of principals |

Facts are read in one statement on the caller's connection (or transaction, for store rechecks). A row that
names an unknown permission or level contributes nothing and is logged as corrupt (never allows).

## Function

```text
custom(S)            = (⋃ perms(r) for r in S) − RESERVED
tenantWide           = perms(builtin) ∪ custom(tenantAssignments)
ceiling(N)           = tenantWide ∪ custom(namespaceAssignments(N))
itemPerms(L)         = LEVEL pairs (permission, kind) conferred by level L (cumulative; owner = editor)
levels(F)            = ⋃ itemPerms(L) for L in entries(F)
readPerm(kind)       = workflow → definition:read, run → run:read, script → script:read,
                       connection → namespace:read
visible(i)           = allowItem(readPerm(kind(i)), i)
visibleFolder(F)     = mode(F) = open       ∧ namespace:read ∈ ceiling(N)
                     ∨ mode(F) = restricted ∧ (entries(F) ≠ ∅ ∨ folder:manage ∈ ceiling(N)
                                              ∨ access:read ∈ ceiling(N))

allowTenant(p)       = p ∈ tenantWide
allowNamespace(p,N)  = p ∈ ceiling(N)
allowItem(p,i)       =   (kind(i) ∉ kinds(p)  → allowNamespace(p, N))
    if p ∈ LEVEL for kind(i):
        folder(i) = root or mode = open → p ∈ ceiling(N) ∨ (p, kind(i)) ∈ levels(folder(i))
        mode = restricted               → (p, kind(i)) ∈ levels(folder(i))
    if p ∈ SENSITIVE for kind(i):       p ∈ ceiling(N) ∧ visible(i)
allowOperationBase(k,run) = allowItem(run:read, run) ∧ allowItem(k = query ? run:query : run:update, run)
allowOperation(k,n,run)   = allowOperationBase(k, run)
                          ∧ (operationList(wf(run),k,n) absent ∨ subject or one of its groups listed)

mayHold(p, kind)     = p ∈ ceiling(N) ∨ ∃ folder F of N: (p, kind) ∈ levels(F)       (no item lookup)
visibleFolders(kind) = { F of N | readPerm(kind) ∈ ceiling(N) ∧ mode(F) = open }
                     ∪ { F of N | (readPerm(kind), kind) ∈ levels(F) }
                     plus the root when readPerm(kind) ∈ ceiling(N)

outcome, in this order (an item or folder operation):
  1. ¬mayHold(p, kind)                   → denied  (403, before any lookup, as today)
  2. target missing, or ¬visible(target) → hidden  (404, the same answer for both)
  3. allowItem(p, target)                → allowed; otherwise denied (403: the caller can see it)
```

Step 1 keeps today's order (`crates/application/src/runs.rs` checks the permission before the lookup), so a
caller who could not act on any such item gets `403` whether or not the target exists; only a caller who
might hold the permission somewhere learns `404`, and then for missing and hidden targets alike. For a tenant
without access objects, step 1 is the built-in role check and nothing changes (FR-029-034).

`visibleFolders(kind)` is the only implementation of visibility for listings: the domain computes the set and
passes it to SQL as a parameter (`folder_id IS NULL AND root visible OR folder_id = ANY($set)`); no SQL
predicate re-derives the rules. A property test checks that an item is listed exactly when `visible(i)`.

`RESERVED`, `LEVEL`, `SENSITIVE`, kinds and `itemPerms` come from [permission-catalog.json](./permission-catalog.json):

| Level | Item permissions conferred (cumulative) |
|---|---|
| `viewer` | `definition:read` (workflow), `script:read` (script), `run:read`, `approval:read` (run), `namespace:read` (connection only) |
| `runner` | + `run:start` (workflow), `run:query`, `run:update` (run), `database:use` (connection) |
| `editor` | + `definition:write`, `definition:publish` (workflow), `script:write`, `script:publish` (script) |
| `owner` | the same item permissions as `editor`, plus administration of that folder's entries |

Every level permission is held by the built-in `developer` role (property test): a level never confers an
operator- or administrator-only power. `run:operate`, `run:signal`, `version:activate`, `database:manage`,
`approval:decide`, `publication:approve`, `run:payload:inspect`, `saga:override`, `database:adhoc`,
`database:write` and `database:result:read` are **sensitive**: ceiling and visibility. Built-in permissions
are never filtered (a `runner` keeps `script:build`). A run's folder is its workflow's current folder.

## Administration rules

```text
admin(actor, F)            = owner ∈ entries(actor, F) ∨ folder:manage ∈ ceiling(actor, ns(F))
admin(actor, root of N)    = folder:manage ∈ ceiling(actor, N)
power(actor, tenant)       = tenantWide(actor)
power(actor, namespace N)  = ceiling(actor, N)
power(actor, folder F)     = ceiling(actor, ns(F)) ∪ levels(actor, F)            (restriction ignored)
powerOn(actor, i)          = { p | allowItem_actor(p, i) }                        (as currently placed)

canGrant(actor, conferred, scope) = conferred ⊆ power(actor, scope) ∧ administers(actor, scope)
canPlace(actor, i, S → D)  = writePerm(kind(i)) ∈ powerOn(actor, i)
                           ∧ admin(actor, S) ∧ admin(actor, D)
                           ∧ (mode(S) = restricted ⇒ folder:manage ∈ ceiling(actor, N))
                           ∧ ⋃ itemPerms(e.level) for e in entries(D) ⊆ powerOn(actor, i)
canCreateIn(actor, kind, F)= createPerm(kind) allowed in F (level or ceiling, as allowItem)
canWidenList(actor, w, k)  = role:assign ∈ ceiling(actor, N) ∧ allowOperationBase_actor(k, runs of w)
                             (the right ADR-0031 would give without the list; being listed is not required,
                             so an access administrator can always widen, audited, and no list locks out)
canAddMember(actor, g)     = group:manage ∈ tenantWide(actor)
                           ∧ ∀ list (w,k,n) naming g: canWidenList(actor, w, k)
                           ∧ ∀ entry naming g on a restricted folder F: admin(actor, F)
```

- An assignment confers `custom({role})` in its scope; a tenant role edit confers the new set tenant-wide;
  a namespace role edit confers it in its namespace; an entry of level L on F confers `itemPerms(L)` on F
  and, for `owner`, administration of F, which requires `admin(actor, F)`.
- A **move** is a grant: the destination's entries start applying to the item (`canPlace`). Creating an item
  in a folder adopts that folder's policy and needs only the create permission there (`canCreateIn`).
- An **operation list**: creating it or removing principals narrows and needs `definition:write` on the
  workflow; adding principals or deleting the list widens and needs `canWidenList`.
- **Adding a member to a group** is a grant of everything that names the group: `canAddMember`, and the
  change is audited as a widening of each list and restricted-folder entry it reaches (so an administrator
  joining a listed group appears in the audit as widening that list, not only as a group change).
- `administers(actor, scope)` is `role:manage`/`role:assign` in that scope for roles and assignments, and
  `admin(actor, F)` for entries. Removals confer nothing and need only `administers`.

## Properties (property tests)

1. Determinism: any permutation of groups, assignments and entries yields the same outcome and sources.
2. Grant monotonicity: adding an assignment, group membership or entry never turns `allowed` into `denied`
   or `hidden`; for a level permission on an item in a restricted folder, only an entry of that folder can
   turn `denied`/`hidden` into `allowed`.
3. Restriction anti-monotonicity: `open` → `restricted` or adding an operation list never turns `denied`
   into `allowed`.
4. Reserved exclusion: no custom role or level yields a permission in `RESERVED`.
5. Built-in floor: with no custom facts and all items at the root, every outcome equals `Role::grants`, and
   step 1 refuses before any lookup exactly where today's code does. Built-in roles are additive, so the
   tests assert set properties (admin holds every assignable classified permission; the five new permissions
   go to admin only; developer holds every level permission), never counts.
6. Developer bound: every level permission is in `perms(developer)`.
7. Kind check: a level never applies a permission to a kind outside its kinds.
8. Listing equivalence: an item is in `visibleFolders(kind(i))` (or at a visible root) exactly when `visible(i)`.
9. No oracle: for every role and fact set, the outcome for a hidden target equals the outcome for a missing
   one.

## Test vectors

Namespace `N`; folders `Fo` (open) and `Fr` (restricted); workflow `wo` in `Fo`, `wr` in `Fr`, `w0` at
root; run `r` of `wr`. Role `rm` = {definition:read, version:activate, run:read, run:operate}.

| # | Facts | Query | Expected |
|---|---|---|---|
| V01 | viewer | `run:start` on `w0` | denied |
| V02 | viewer; entry `Fo`: runner | `run:start` on `wo` | allowed (folder level) |
| V03 | viewer; entry `Fo`: runner | `run:start` on `w0` | denied |
| V04 | developer | `definition:read` on `wr` | hidden |
| V05 | developer; entry `Fr`: viewer | `definition:publish` on `wr` | denied |
| V06 | viewer; entry `Fr`: editor | `definition:publish` on `wr` | allowed (folder level) |
| V07 | admin | `definition:read` on `wr` | hidden |
| V08 | admin | grant self `Fr`: owner | allowed (`admin(actor,Fr)` via folder:manage; editor items ⊆ ceiling) |
| V09 | operator; entry `Fr`: viewer | `run:payload:inspect` on `r` | allowed (ceiling ∧ visible) |
| V10 | viewer; entry `Fr`: owner | `run:operate` on `r` | denied (sensitive, not in ceiling) |
| V11 | viewer; `rm` assigned in `N` | `run:operate` on a run of `w0` | allowed (custom) |
| V12 | viewer; `rm` assigned in `N` | `run:operate` in namespace `M` | denied |
| V13 | viewer; group g; `rm` assigned to g in `N` | `version:activate` on `w0` | allowed (custom via group) |
| V14 | any; role {worker:execute}, {payload:read-internal}, {worker:bind} or {secret:manage} | definition | `RESERVED_PERMISSION` |
| V15 | developer + {role:assign} in `N` | assign `rm` in `N` | `PRIVILEGE_ESCALATION` (lacks version:activate, run:operate) |
| V16 | operator + {role:assign} in `N` | assign a {run:operate} role in `N` | allowed |
| V17 | operator + {role:assign} in `N` | the same at tenant scope | `PRIVILEGE_ESCALATION` |
| V18 | developer; list `update/approve` = {group h} | Update `approve` on a run of `w0` | denied `OPERATION_NOT_GRANTED` |
| V19 | developer in h; same list | Update `approve` | allowed |
| V20 | viewer in h; same list | Update `approve` | denied (`run:update` not held) |
| V21 | developer; list only on `update/approve` | Query `status` | allowed |
| V22 | developer (no role:assign) | add self to list `update/approve` | `PRIVILEGE_ESCALATION` (widening) |
| V23 | developer; entry `Fr`: viewer by group, editor by subject | `definition:write` on `wr` | allowed (max level) |
| V24 | developer; entry `Fo`: viewer | `definition:write` on `wo` | allowed (ceiling; open folders never narrow) |
| V25 | developer + {folder:manage}; `Fo` gives group G editor | move `w0` → `Fo` | allowed (editor items ⊆ developer) |
| V26 | viewer + {folder:manage, definition:write}; `Fo` gives G runner | move `w0` → `Fo` | `PRIVILEGE_ESCALATION` (run:start, run:query, run:update, database:use not held on `w0`) |
| V27 | developer, owner of `Fr` and `Fo`, no folder:manage | move `wr` → `Fo` | `PRIVILEGE_ESCALATION` (restricted source) |
| V28 | developer | `getFolder Fr` | hidden |
| V29 | viewer; entry `Fr`: viewer | `run:start` on `wr` | denied `403` before lookup (no ceiling or level confers `run:start`) |
| V30 | runner (machine) | any custom fact rejected at write | `MACHINE_IDENTITY` (or `PRINCIPAL_NOT_ELIGIBLE` without `access:read`) |
| V31 | viewer, no entries | `run:operate` on hidden `r` | denied `403` before lookup (step 1) |
| V32 | viewer, no entries | `run:operate` on a missing run id | denied `403` before lookup, identical to V31 |
| V33 | operator | `run:operate` on hidden `r` | hidden `404`, identical to a missing run id |
| V34 | viewer; entry `Fo`: runner | `run:start` on hidden `wr` | hidden `404` (mayHold via `Fo`), identical to missing |
| V35 | viewer; entries `Fo`: editor, `Fr`: viewer | `definition:publish` on `wr` | denied `403` after lookup (mayHold via `Fo`; `wr` visible; the `Fr` level lacks publish) |
| V36 | admin, not on list `update/approve` naming group h | add self to h | allowed (`canWidenList` holds through `role:assign` and `allowOperationBase`); audited as `group.member_added` and `operation_list.widened` |
| V37 | developer + {role:assign} in `N`, not on an empty list `update/approve` | add self to the list | allowed, audited `operation_list.widened` (an empty list can still be widened) |
