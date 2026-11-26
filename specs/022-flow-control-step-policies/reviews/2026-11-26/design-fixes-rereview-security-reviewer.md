# Security re-read of the third-read fixes of the step-policies design (2026-11-26)

Short re-read of the fixes alone (OD-2026-11-25-2 step 4): commit `1276d5b7` of `feature/022-step-policies`, against
`reviews/2026-11-25/design-third-reads-and-rulings.md` (ruling 7, the `RunRecovery` role, security Majors 1 and 2; ruling 11,
security item A). The third-read security report itself is not in the tree; the reviewer judged each element of the rulings and
checked them against the code the design plugs into.

Verdict: Major 2 and item A fixed; Major 1 partly fixed (the binding is stated, not where it is enforced, so it fails open).
One new Major, three minors, three nits, one suspicion.

| Finding | Verdict | Evidence |
|---|---|---|
| Major 1, part 1: scope bound to named namespaces, opaque `404` outside | partly | Stated in ADR-0136 lines 73-74, `plan.md:1024-1025`, `spec.md:582-584`, `contracts/errors.md:43`, `data-model.md:791-794`, task T022-253; not where it is enforced, nor that no binding row means no namespace. A principal carries no namespace today (`crates/domain/src/identity.rs:742-785`) and `admit` checks permissions before resolving the namespace (`crates/application/src/api_admission.rs:274-324`). New finding 1. |
| Major 1, part 2: who grants the binding, its audit | partly | "An administrator's audited setting" (ADR-0136 line 73, `plan.md:1029`); no route, permission, audit action or error rows. New finding 3. |
| Major 1, part 3: what the role can and cannot do | fixed | `plan.md:1022-1027`; `definition:read` added (ADR-0136 line 75). |
| Major 1, part 4: payloads, secrets, failure text | fixed (no access) | Holds `run:read`, `run:start`, `run:operate`, `definition:read` only; none of `payload:read-internal`, `run:payload:inspect`, `secret:*`, `run:logs:read`, `event:read`; `payload:reveal-pii` never granted by role (`identity.rs:313-315`); bodies carry codes and references (`crates/api-http/src/dto_exec.rs:488-533`, `:651-673`). |
| Major 1, part 5: widening by a machine identity | fixed (cannot) | Role from the database, not token claims (`crates/api-http/src/core.rs:35`); one role per tenant (`identity.rs:737-746`); lacks `membership:manage`, `worker:bind`, `schedule:write`, `run:batch`, `run:signal`, `run:update`. |
| Major 2: the role may not redrive a dispatch | fixed | ADR-0136 line 75, `plan.md:1026`, `errors.md:42`, `openapi-flow-control.yaml:323`, fault test `plan.md:1084`. |
| Major 2, related: role not assignable before `failure-handler-v1` is activated | partly | ADR-0136 lines 75-76, `plan.md:1030`; no error row for the refused assignment; switching the capability off again is not described. |
| Security item A (ruling 11): machine-identity leftovers | fixed (one nit) | Struck in `plan.md:1032-1034`, `:1082-1083`, `data-model.md:669`, `:679`, ADR-0136, `design-reresponses.md`; `tasks.md:1072` left (new finding 7). |

## New findings

1. **Major: the binding is a rule written per route, so any route that forgets it fails open.** ADR-0136 line 74,
   `plan.md:1024-1025`, task T022-253 say "every route that authorizes it checks the binding first", yet about 44 sites in
   `crates/application/src` call `principal.authorize(Permission::RunRead | RunStart | RunOperate | DefinitionRead)` with no
   namespace, and `admit_resolved_namespace` takes no principal (`api_admission.rs:332-336`). A `RunRecovery` subject bound only to
   `staging`, whose credential a `staging` author can obtain from a handler's secret, calls `PUT /v1/run-input-blobs` with the
   namespace header `prod`: `input_blob_access` checks only `run:start` and the tenant (`crates/application/src/runs.rs:978-979`)
   and `crates/api-http/src/blobs.rs:118-122` admits it. The planned evidence (one battery case) would not catch missed sites.
   Fix: load the bound namespace set into `Principal` with the membership, no row meaning no namespace; `Principal::authorize`
   refuses every namespace-scoped permission for `RunRecovery`; a new `authorize_in(namespace, permission)` called from
   `require_namespace`, `resolve_run`, `resolve_workflow` and the use-case checks; the tenant battery over every route as a
   `RunRecovery` subject against an unbound namespace (or a rule in `scripts/check-architecture.py`).
2. **minor: the new table's grants contradict their precedent.** `data-model.md:791-794` grants only SELECT to `sagat_app`,
   though the administrator's route runs as `sagat_app`; worker bindings (the precedent) grant SELECT, INSERT, UPDATE, DELETE,
   with a composite foreign key `(tenant_id, namespace_id)` to `namespaces`, a CHECK on `subject` and quota triggers
   (`20261010000100_worker_bindings.up.sql:13-25`, `20261024000000_durable_quotas.up.sql:532-535`). Fix: SELECT, INSERT and
   DELETE; the composite foreign key with ON DELETE CASCADE; the subject CHECK; the quota triggers.
3. **minor: the grant surface is not specified:** no route, permission, audit action or error rows. Fix: `membership:manage`
   (administrator only), the route, the audit action and the error rows in the OpenAPI fragment and `contracts/errors.md`.
4. **minor: an attempt outside the binding is not audited** (`errors.md:43`). Fix: audit it like `record_cross_tenant`
   (`api_admission.rs:368-375`), with a metric of fixed labels.
5. **nit: the redrive description does not state that `404` outside the binding comes before `403
   RECOVERY_ROLE_MAY_NOT_REDRIVE`** (`openapi-flow-control.yaml:323`).
6. **nit: the activation gate has no error row and no rule for switching the capability off.**
7. **nit: `tasks.md:1072`** (task T022-228's base evidence) still says "a handler that reruns its failed run through a machine
   identity".

## Suspicion (not checked in the code)

A replay after a revocation: the design does not say the binding is read on every request, nor that an idempotent replay of a
start or operation checks it again (FR-022-032: "replays MUST reauthorize"). Check where `start_run` looks up the receipt relative
to `admit` (`crates/application/src/runs.rs` from line 986).

The other fixes (D022-260 to D022-265, D022-269) open no hole in the tenant boundary, the runtime role's privileges, or an
unauthenticated or unaudited path.
