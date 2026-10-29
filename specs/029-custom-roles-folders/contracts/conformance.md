# Conformance, batteries and scans (draft)

## Shared five-SDK scenario `access-control`

File: `tests/contract/sdk-conformance/scenarios/access-control.json` with expected observations in
`tests/contract/sdk-conformance/expected/`. The harness provisions the access objects through REST as an
administrator (SDK clients do not manage access: the client surface of `sdks/SPEC.md` covers runs,
approvals, events and declared Query/Update calls, not administration); each SDK then acts as the named
member through its public client API. Normalized observations must match across TypeScript, Go, Python,
C# and Java.

| Step | Actor | Call | Expected observation |
|---|---|---|---|
| A1 | viewer with custom role {run:start} in N | start run of `w0` | run id, status `RUNNING` or later |
| A2 | same | start run in namespace M | `FORBIDDEN`, retryable false |
| A3 | developer, not on restricted folder `Fr` | get run of `wr` | `NOT_FOUND`, no `details.reason`, same message as a random id |
| A4 | developer, not on restricted folder `Fr` | list runs | no run of `wr` in any page |
| A5 | operator, not on list `update/approve` | Update `approve` | `FORBIDDEN`, `details.reason = OPERATION_NOT_GRANTED` |
| A6 | operator, on the list | same Update, same key | applied once; retry replays the receipt |
| A7 | admin removes A1's assignment; viewer retries A1 | start run | `FORBIDDEN` on the first call after the removal commits |
| A8 | developer who started a run of `wr` with key k before `Fr` was restricted and is not listed | start again with k | `IDEMPOTENCY_KEY_REUSED`; the stored run id is not returned |
| A9 | any | SDK error mapping | the reasons of contracts/README.md map to existing error types; no new SDK error code |

The scenario runs against the real engine with canned worker answers (feature 012 pattern). It proves
client-visible outcomes only; the authorization rules are proven by the server suites.

## Opacity battery (per item kind)

`tests/contract/custom-roles-folders/opacity.rs`, modelled on `tests/contract/security-multitenancy/rest/battery.rs`
(D50). For each item kind being enabled, for every operation whose `x-access` names the kind (`kind`) or
discloses it (`discloses`), and for folder operations:

1. Call as each built-in human role (viewer, developer, operator, admin) and as a level-only member, each
   hidden by a restricted folder, and as the same caller with a random id or name; require equal status,
   body (ignoring `traceId`/`requestId`), headers except trace and request ids, and equal rate-limit effect
   (the namespace rate bucket is charged identically). A caller without the permission on that kind gets
   `403` for both (refused before any lookup); one who may hold it gets `404` for both.
2. Listings and disclosing operations must omit the hidden item and folder.
3. Name-keyed writes (create, upsert by name, promotion target, manifest publish) against a hidden name must
   answer as the operation's ordinary collision, and the hidden item's rows (revision, draft, state) must be
   byte-identical before and after.
4. Idempotent replays of keys whose stored response names the hidden item must not return it.

An operation that names or discloses the kind without a battery entry fails the test.

## Canary scan

`tests/contract/custom-roles-folders/canary_scan.rs`: create a hidden canary workflow, run, script and
connection with distinctive names and recorded identifiers; as a hidden developer, call every implemented
operation (namespace-class included: alerts, event inbox and dead letters, workers, publication requests,
quotas, script executions) and scan bodies, headers and SSE frames for any canary name or identifier. Any
hit fails the gate for that kind (catches a route with a wrong `class` or missing `discloses`). Audit reads
(`GET /audit` and any audit export) are excluded by an explicit list, because audit readers see past
restriction by design (G1451); the exclusion list is itself asserted, so a new operation cannot hide in it.

## Escalation matrix

`tests/integration/custom-roles-folders/escalation.rs`: granter permission sets {each built-in human role;
each plus `role:assign`/`role:manage`/`folder:manage` by namespace or tenant assignment; a folder owner with
a viewer base} × grant kinds {tenant-role create/edit, namespace-role create/edit, tenant assignment,
namespace assignment, folder entry per level, create in folder, move root→folder, move open→open, move
restricted→open/root, manifest placement, list create, list narrow, list widen, list delete, group member
addition (group named by a list and by a restricted entry)} × scopes {own
namespace, other namespace, tenant}. Each cell's expected outcome is computed by `canGrant`, `canPlace`,
`canCreateIn`, `canWidenList` or `canAddMember` from evaluation.md and checked against the HTTP answer and the presence or
absence of the row and its audit record (SC-029-003).

## Coverage test

`tests/contract/custom-roles-folders/coverage.rs`: every operation of the canonical OpenAPI document has a
valid `x-access`; every response schema with item identifiers or names declares `discloses`; the route
registry generated from typed extractors equals the declarations; `restrictedFolders` and `operationLists`
report no kind whose battery or canary scan is incomplete (SC-029-005).
