# Security re-read of the fourth re-read fixes (2026-11-27, commit `1b2547d5e`)

Short re-read (OD-2026-11-25-2 step 4) by the security reviewer, against
[design-fixes-4-rereview-security-reviewer.md](design-fixes-4-rereview-security-reviewer.md) and the "Fourth re-reads
(2026-11-27)" section of [design-fixes-responses.md](design-fixes-responses.md).

## Verdict

Commit `1b2547d5e` fixes every item of the fourth security re-read, and the code premises the decisions cite hold. One fix is incomplete. D022-317 puts the namespace-bound filter on the schedule's unlocked membership read, but leaves out the in-transaction re-check that actually fences a start. A demotion to `run-recovery` that commits between that read and the start transaction therefore still starts one run. The ruling to use `stop_backfill` in the backfill path is safe, and better than `revoke_authority` (reasons in the table). There is no Blocker and no Major. The fixes bring two new minors and three nits. New minor 1 is a security finding: its fix is one condition in `recheck_authority` and one held-lock test in T022-270. It does not block accepting the ADR amendments or starting the build, because T022-270 lands in slice 13. Because it is a security finding, it should be written into D022-317 and T022-270 before slice 13 starts; it also supplies the missing argument for the `schedules::recheck_authority` allow-list entry. The other minor and the nits can be fixed in the tasks they belong to.

Paths are relative to `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/`.

## Items of the fourth report

| Item | Verdict | Evidence |
|---|---|---|
| Minor 1 (wildcard arm in the conversion) | Fixed | D022-315 (`research.md:3429-3451`), T022-268 (`tasks.md:1282`). The seven variants T022-268 lists are exactly the enum's (`crates/domain/src/identity.rs:32-55`, not `#[non_exhaustive]`). The only place that builds `AppError::Invalid` is the wildcard itself (`crates/application/src/error.rs:366`; grep finds no other), so no path bypasses the conversion. T022-257 now asserts the status `404` (`tasks.md:1272`). |
| Minor 2 (denial record never triggered) | Fixed, with new minor 2 and nit 3 | D022-316 (`research.md:3452-3472`), `data-model.md:918-926`, T022-269 (`tasks.md:1283`). The premise holds: `record_cross_tenant` is async and takes the request context (`crates/application/src/api_admission.rs:457-479`), so a `From` cannot queue it. A miss and a hit now take the same response path. |
| Minor 3 (`pay_obligation` reads `NotFound` as "done") | Fixed as reported; the lock it relies on is missing (new minor 1) | D022-317 (`research.md:3473-3496`), T022-270 (`tasks.md:1284`). The filter sites are as cited (`crates/application/src/schedule_firing.rs:759-772`, `:454-463`). The `trigger_once` claim holds: the route needs `schedule:operate` (`:221`), which the role does not hold. |
| The `stop_backfill` ruling | Safe | It is the branch the code already takes when a requester has no membership or lost `run:start` (`schedule_firing.rs:462-463`), and when the start transaction answers `AUTHORITY_REVOKED` (`:612-613`). It is terminal, starts nothing, and is audited `ScheduleBackfillCancelled` "authority revoked" (`:1642-1660`). `revoke_authority` would pause the whole schedule because its backfill requester was demoted. But the schedule acts for `authorized_by`, a different subject that `fire` still checks. Pausing it would wrongly penalise the schedule's owner. One older caveat, not new: `cancel_backfill` takes no shard fence (`:1657`), but it can only stop work. |
| Nit 4 (allow-list names) | Fixed | D022-318 (`research.md:3497-3515`). `evaluate` at `crates/application/src/payload_disclosure.rs:619` is inside `impl DisclosureEvaluator for ServiceEvaluator` (`:600`), and `:311` is the trait declaration (`:303`). `authorize` is a free function (`crates/infra-postgres/src/declared_operations.rs:218`, outside the `impl` at `:35`). |
| Nit 5 (`.permissions()` rule) | Fixed, with nit 5 below | D022-319 (`research.md:3516-3530`). The caller list is complete: `crates/domain/src/identity.rs:710`, `crates/api-http/src/routes/identity.rs:51`, `declared_operations.rs:252`, `crates/infra-postgres/src/execution.rs:1394`, and the domain tests at `crates/domain/tests/identity.rs:365` and `:487`. Every other `.permissions()` in the tree is on file metadata, `operation` or `entry`. |
| Nit 6 (direct delete not audited; runbook) | Fixed, with nit 4 below | D022-320 and D022-321 (`research.md:3531-3553`), `data-model.md:914-916`, T022-266 and T022-267 (`tasks.md:1277-1278`). |
| Nit 7 (code with no producer; two foreign keys share `23503`) | Fixed | D022-322 and D022-323 (`research.md:3554-3571`). Premise checked: provisioning only inserts (`crates/infra-postgres/src/identity.rs:146-185`), and every `UPDATE memberships` in the tree is in tests or a runbook. Each remaining mention of `RUN_RECOVERY_BINDINGS_EXIST` is struck through. The namespace key is composite `(tenant_id, namespace_id)` (`data-model.md:912`), so another tenant's namespace gets the same opaque `404` as a missing one (`contracts/errors.md:52`). `NOT_A_RECOVERY_SUBJECT` tells a tenant administrator nothing they cannot already see. |

## New findings

### 1. minor (security, race): the namespace-bound check is not inside the transaction that fences the start

Verified in the code. `recheck_authority` (`crates/infra-postgres/src/schedules.rs:175-206`) is the check FR-018-015 relies on: "a revocation that is already committed stops the run". It locks the member through `sagat_lock_memberships` (`:182`), then tests:
```rust
role.grants(Permission::RunStart) && database.is_none_or(..)
```
`run-recovery` holds `run:start`. Every schedule start runs this check through `recheck_schedule_authority` (`crates/infra-postgres/src/execution.rs:1403-1421`, called at `:1532`, `:1542`, `:1653`): a firing, a backfill step, and the trigger-now of a person.

**Scenario.**
1. Subject X, a Developer with no binding, authorized schedule S in namespace B.
2. `fire` reads X's membership with no lock (`schedule_firing.rs:759-768`), sees Developer, and the new filter passes. It builds a tenant-wide principal from that stale role (`:774`).
3. An operator commits X's change to `run-recovery`. Nothing refuses it, because D022-298 refuses a role change only while a binding exists.
4. The start transaction locks the membership, reads `run-recovery`, finds `holds = true`, and admits a run in namespace B under the authority of a subject with no binding to B.

The backfill step (`:454-459`, then `:606`) and a Developer's trigger-now (`:321-339`) have the same window. The effect is one run per window: the next firing revokes, and database steps fail closed because the role lacks `database:use`. Still, D022-317's argument for the `ScheduleFiring::fire` and `ScheduleFiring::backfill_step` entries ("a namespace-bound role never reaches their checks") is true only of the unlocked read. The `schedules::recheck_authority` entry that D022-287 lists (`research.md:3062`) still has no argument.

**Fix.** In `recheck_authority`, test `role.grants(Permission::RunStart) && !role.is_namespace_bound() && ...`. The existing `AUTHORITY_REVOKED` branches then do the rest: `revoke_authority` (`schedule_firing.rs:1306`), `stop_backfill` (`:612`), and the trigger-now answer (`:342`). Add a held-lock case to T022-270: hold the role update open, let `fire` pass its filter at a barrier, commit, then assert `AUTHORITY_REVOKED` and that no run was created. Record this as the argument of the `schedules::recheck_authority` entry in T022-258.

### 2. minor (audit): D022-316 narrows the denial task's existence query to the tenant, so a bound principal's cross-tenant attempt is no longer specified

Verified in the design. D022-294 (`research.md:3126-3128`) said the task's work covers "the cross-tenant existence query included". D022-316 (`research.md:3462-3463`) and the data model (`data-model.md:922-923`) now say the task reads existence "in the principal's tenant". Today the resolvers run `*_exists_outside` inline and write `CrossTenantAccessDenied` (`crates/application/src/api_admission.rs:412-424`, `:441-453`). The design does not say whether that inline check still runs for a bound principal.

**Scenario.** A subject bound to namespace A in tenant T reads a run identifier that exists in tenant U. If T022-269 follows the text, no `cross_tenant.access_denied` row is written, though any other principal's identical attempt is audited. If the implementer keeps the inline check, D022-294's off-the-response-path property is lost again for that case. No data leaks either way.

**Fix.** State it in D022-316 and T022-269. For a bound principal the resolver skips the inline `*_exists_outside`. The task checks existence without the scope predicate and without the tenant predicate. It writes `run_recovery.namespace_denied` for a hit in the tenant and `CrossTenantAccessDenied` for a hit in another tenant. Add an other-tenant identifier case to T022-269 that asserts one cross-tenant row.

### 3. nit: a hand-off dropped under load is neither audited nor counted

Today `record_denial` increments `security_denials_total` before it tries for a slot, and counts `audit_denials_total{outcome="dropped"}` when none is free (`crates/infra-postgres/src/identity.rs:589-598`). D022-316 increments `run_recovery_namespace_denials_total` only inside the task, after the existence query. It also does not say whether the hand-off shares `denial_slots`. If the semaphore is full, a real attempt outside the scope disappears without a trace. Fix: T022-269 says the hand-off takes the same `denial_slots`, and that a drop increments `audit_denials_total{outcome="dropped"}`.

### 4. nit: the unbind marker departs from its precedent and is never cleared

The precedent sets `sagat.registry_change` inside a security-definer function and clears it right after the protected statement (`crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:342`, `:348`, `:370`, `:375`). D022-320 sets the marker from application SQL and leaves it on until commit. Any later delete of a binding in the same transaction, with both parents present, then goes unaudited. Fix: the route runs `set_config('sagat.unbind_audited', '', true)` right after its delete. Alternatively, do the unbind in a security-definer function that marks, deletes, audits and clears.

### 5. nit: the `.granted_permissions(` rule exempts more than the `.grants(` rule

D022-319 refuses `.granted_permissions(` "outside `crates/domain`". The `.grants(` rule exempts only `crates/domain/src/identity.rs`, and lists domain sites such as `execution/approval.rs` `authorize_decision` and `secrets.rs` `may_use` (`research.md:3054-3059`). So a `role.granted_permissions().contains(..)` written in those files would escape both rules. Fix: exempt only `crates/domain/src/identity.rs`.

## Relevant files
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/schedules.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/schedule_firing.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/error.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/api_admission.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/domain/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/data-model.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md
