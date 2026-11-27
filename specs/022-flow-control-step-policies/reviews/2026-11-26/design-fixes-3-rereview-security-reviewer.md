# Security re-read of the fixes of the second security re-read (2026-11-27, commit `5ef574f16`)

Short re-read (OD-2026-11-25-2 step 4) by the security reviewer, against
[design-fixes-2-rereview-security-reviewer.md](design-fixes-2-rereview-security-reviewer.md) and the "Second security
re-read" section of [design-fixes-responses.md](design-fixes-responses.md).

Checked by hand before answering: new finding 1 holds. On `postgres:16-alpine`, a role granted SELECT, INSERT and DELETE
on a table is refused `SELECT ... FOR SHARE` and `FOR KEY SHARE` ("permission denied"), and is admitted once granted
UPDATE.

**Verdict.** Commit `5ef574f16` handles every item of the earlier report. Seven are fully fixed and four are partly fixed; none is left unfixed. I checked each premise the fixes rely on against the code, and all of them hold, with the exceptions listed below. The fixes add no Blocker and no Major, but they bring four new minors and four nits. The most serious minor: the design locks `run_recovery_namespaces` rows `FOR SHARE` in three places (D022-277, and twice in D022-288), but grants the runtime role no UPDATE on the table and gives it no locking function. That statement cannot run as `sagat_app`, so the revocation fence is impossible to build as written. It fails closed (permission denied), which is why it is a minor and not a Major. The second minor: D022-288's new `AppError::OutsideNamespaceScope` is not matched by the event stream's loop, so D022-296's 30-second window does not hold.

Paths below are relative to `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/`.

## Items of the earlier report

| Item | Verdict | Evidence |
|---|---|---|
| Minor 1: a role change leaves the bindings in place | Fixed | D022-286 (`research.md:3023-3031`), data model `:845-852`, T022-261. It copies the precedent exactly (`migrations/20261220000000_reveal_grants_role_change.up.sql:11-27`). The bind reads the role through `sagat_lock_memberships`, which `sagat_app` may execute (`migrations/20261126000000_authority_read_locks.up.sql:4-10`, `:30-32`). Residual: see nit 2 and the suspicion. |
| Minor 2: the fitness rule matches four names; `Membership::restore` is public; the role-only method has no name | Partly fixed | The allow list in D022-287 (`research.md:3039-3044`) matches every production `.grants(` site in the tree, enclosing function by function (checked against grep). `Membership::restore` is confined, and `check_role_before_lookup` is named. **The remaining gap:** `Role::permissions()` is public (`crates/domain/src/identity.rs:618`), and two role-only checks are written as `.permissions().contains(` rather than `.grants(`. They are `crates/infra-postgres/src/declared_operations.rs:252` (function `authorize`, called with `run:read` at `:845-851`) and `crates/infra-postgres/src/execution.rs:1392-1395`. The rule as written does not see them. They are safe today only because declared operations also need `run:query` or `run:update`, which the role lacks. Fix: T022-258 also refuses `.permissions()` on a `Role` in production code outside `identity.rs`, allow-listing only `auth_context` (`crates/api-http/src/routes/identity.rs:49-54`); or rewrite those two sites with `.grants(`. Also, `payload_disclosure.rs` has two functions named `evaluate` (`:311`, `:619`), so an allow-list key of "file plus function name" also admits the one at `:311`; key the allow list by `impl` as well. |
| Minor 3: the pending reset completion is not covered | Partly fixed | The intent is right: the premises (`pending_reset.rs:485-488`, `reset_completion.rs:225`, `:255`, `:361`, `resets.rs:655-681`) hold, and `PendingReset.namespace_id` exists (`pending_reset.rs:93`). The means have two defects: the binding read `FOR SHARE` in `recheck_requester` cannot run as `sagat_app` (new finding 1), and the new `AppError` variant is not handled where `NotFound` is (new finding 2). Also, `PendingAdmission` (`pending_reset.rs:138-146`) carries no namespace; it should gain `entry.namespace_id`. |
| Minor 4: the battery accepts "403 or the opaque 404" | Fixed | T022-257 as it reads now (`tasks.md:1211`), with the comparison taken from `tests/contract/security-multitenancy/rest/battery.rs:7-8`. |
| Minor 5: the activation trigger's function attributes are unspecified | Fixed | D022-290; data model `:848-850`; T022-253 and T022-261 add a `pg_proc` check and a test that `sagat_app` cannot execute the function. This matches the precedent at `reveal_grants_role_change.up.sql:11-27`. Revoking EXECUTE does not stop the trigger firing (PostgreSQL checks EXECUTE only when the trigger is created). |
| Minor 6: merge order could open a window | Fixed | D022-291 (a) and (b); T022-249 names the artefacts (`tasks.md:1208`); the CHECK widening lands with T022-256 (`tasks.md:1220-1221`). Residual: see new nit 4. |
| Nit 1: the scope class of the role's permissions is not pinned | Fixed | The domain test is in the T022-256 row (`tasks.md:1210`). |
| Nit 2: "before any other refusal of the route" | Partly fixed | Now reworded to "after the role check" (`contracts/errors.md:44`). It still says "before every other refusal", but `start_with` checks the body shape (`crates/application/src/runs.rs:1361-1390`) before it resolves the namespace (`:1391`). That is not an oracle, because the check does not depend on the namespace. Reword to "before every refusal that depends on the addressed resource". |
| Nit 3: response time tells existing slugs | Fixed | The premise is right: the denial write is off the response path (`crates/infra-postgres/src/identity.rs:593-614`), and the `*_exists_outside` queries are awaited on it (`crates/application/src/api_admission.rs:370`, `:390`, `:414`, `:443`). Slugs are unique per tenant (`migrations/20261008000000_identity.up.sql:37`), so a check against the bound set before any lookup is sound. Residual: see new nit 3. |
| Nit 4: "machine identity" left in `tasks.md:1073` | Fixed | The amendment is struck whole (`tasks.md:1073`). |
| Suspicion: the stream lifetime sets the revocation window | Partly fixed | The premises check out: 15 minutes at `crates/api-http/src/streams.rs:49`, 30 seconds at `:50`, `crates/api-http/src/routes/execution.rs:1276`, `:1304`, `:1385-1400`, `crates/application/src/runs.rs:2430`. But the loop ends only on `NotFound` or `Forbidden` (`execution.rs:1408`, `:1437`), so the 30-second claim depends on new finding 2. |

## New findings

1. **minor: the runtime role cannot take the binding row lock the design relies on.** A row lock needs UPDATE privilege; the codebase says so at `crates/infra-postgres/src/declared_operations.rs:236-238`, and every membership lock goes through the security-definer function `sagat_lock_memberships` for this reason (`crates/infra-postgres/src/execution/resets.rs:652-661`). D022-273 grants `sagat_app` only SELECT, INSERT and DELETE on `run_recovery_namespaces`, and explicitly rejects a security-definer function (`research.md:2910-2913`, data model `:817`, plan `:1084`). Yet three places read the row `FOR SHARE`: the slot-4a read (D022-277, ADR-0050 decision 7 at `:141-143`, T022-259), the reset re-check (D022-288, ADR-0050 `:144-145`, T022-262) and data model `:855-856`.
   - Scenario: a `RunRecovery` start runs as `sagat_app` and fails with "permission denied for table run_recovery_namespaces". Every admission or command by the role becomes an internal error. It fails closed, hence a minor.
   - Risk: an implementer "fixes" it by dropping `FOR SHARE`, which reopens the revocation window D022-277 closed, or by granting UPDATE.
   - Fix: add `public.sagat_lock_run_recovery_binding(p_tenant uuid, p_subject text, p_namespace uuid)`, `LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp`, `... FOR SHARE OF b`, `REVOKE ALL ... FROM PUBLIC`, `GRANT EXECUTE ... TO sagat_app`, written like `migrations/20261126000000_authority_read_locks.up.sql:4-10`. Amend D022-273's rejection, and make T022-259 and T022-262 run as `sagat_app`.

2. **minor: the new `AppError::OutsideNamespaceScope` misses the existing `NotFound` matches.** D022-288 (`research.md:3056-3058`, plan `:1069`) makes the scope refusal an `AppError` variant of its own. The HTTP mapping is an exhaustive `match` (`crates/api-http/src/error.rs:725`), so the compiler forces a mapping there. The stream loop is not exhaustive: `Err(AppError::NotFound | AppError::Forbidden { .. })` ends the stream, and anything else counts a tick error and `continue`s (`crates/api-http/src/routes/execution.rs:1408-1415`, `:1437`).
   - Scenario: a `RunRecovery` subject streams a run in namespace A, and an administrator unbinds A. At the next re-check, `authenticate` succeeds (the membership still exists, `:1387-1388`), and `RunService::get` through `authorize_in` returns the new variant. The stream sends nothing more but stays open, holding its per-subject slot, until the 15-minute deadline.
   - No data leaks, but D022-296, the ADR-0136 revision (`:97`) and the spec (`spec.md:604-605`) promise 30 seconds.
   - Fix (the smallest, with no new variant): map `IdentityError::OutsideNamespaceScope` to `AppError::NotFound`, with the denial attached. In `complete_reset`, test the scope before the role and return `AppError::StateConflict(AUTHORITY_REVOKED)`, which `failure_of` already maps (`crates/application/src/reset_completion.rs:226`).
   - If the variant stays instead: add it to both stream arms and review every `AppError::NotFound` match the role can reach.

3. **minor: the allow list admits an entry its own criteria cannot argue, and payload disclosure is not fenced against a revocation.** D022-287 lets an entry through only for "a permission no namespace-bound role holds, or a stored role checked again in a transaction that also reads the binding". `payload_disclosure.rs` `evaluate` checks `self.subject.read`, which is `run:read` (`crates/application/src/payload_disclosure.rs:455`, `:625`). The role holds `run:read`, and the disclosure transaction re-reads the role under `sagat_lock_memberships` (`crates/infra-postgres/src/payload_disclosure.rs:108-109`, `:401`, `:585-599`) but not the binding.
   - Scenario: a `RunRecovery` payload read in namespace A authenticates, then the binding is revoked before the disclosure transaction. A demoted member would be refused at that point; this subject gets the payload. The window is one request, but payload disclosure is the one read the platform deliberately fences in-transaction.
   - Fix: when the role is namespace-bound, the disclosure transaction also takes the binding row of the run's namespace through the function of finding 1, and answers the opaque `404` when it is gone. Add a T022-259 case for it.

4. **minor (the remainder of minor 2 above is counted there, not here).** No further minor.

## Nits

1. **A role change deletes bindings without an audit row.** D022-274 audits every unbind (`run_recovery.unbound`), but the trigger of D022-286 deletes rows silently. The administrator's history then shows a bind with no end (the reveal-grant precedent has the same gap). Fix: the trigger function writes one `run_recovery.unbound` audit row per row it deletes, with a provisioning actor (`DELETE ... RETURNING` into `public.audit_records`).
2. **`check_role_before_lookup` "returning `()`"** (plan `:1068`, `research.md:3045`): taken literally, it cannot refuse, which would lose the resolvers' `403`. State `Result<(), IdentityError>` and `#[must_use]`.
3. **"Same work on the response path" is checked by a call log, but the rows differ.** For a bound principal, a run in namespace B in the same tenant is fetched and decoded in full by `store.get(tenant, id)` (`crates/application/src/runs.rs:2431`), while a miss fetches nothing. The T022-256 test, which compares the statements called, cannot see that difference, yet the spec promises "in time" (`spec.md:604`). Fix: for a bound principal, put the scope in the lookup itself (`AND namespace_id = ANY($bound)`), so a hit outside the scope returns no row, as the tenant filter does across tenants.
4. **Which slice-2 task carries the empty-scope mechanism is unclear.** The T022-011 and T022-240 row says the role is namespace-bound from slice 2 (`tasks.md:1207`). But `Role::is_namespace_bound`, the exhaustive scope class and the refusal in `Principal::authorize` are still listed only in T022-256 (`tasks.md:1187`), which lands in slice 13. Name which of these move to slice 2.

## Suspicion (reasoned from PostgreSQL semantics, not run)

The bind and role-change race closes only when the role change runs at READ COMMITTED. Suppose the operator's `UPDATE memberships SET role = ...` runs at REPEATABLE READ. It waits on the bind's `FOR SHARE` and then proceeds: the row was only locked, not updated, so there is no serialization error. The trigger's `DELETE` then uses the transaction's old snapshot, misses the binding the bind just committed, and minor 1's scenario comes back. Role changes happen out of band (every `UPDATE memberships` in the tree is a test), so their isolation level is the operator's choice. Fix: the trigger function raises unless `current_setting('transaction_isolation') = 'read committed'`, or the runbook says so; add a T022-261 case at REPEATABLE READ.

To check it, in two `psql` sessions against a migrated database:
- Session A: `BEGIN; SELECT * FROM sagat_lock_memberships('<tenant>', ARRAY['<subject>']); INSERT INTO run_recovery_namespaces ...;`
- Session B: `BEGIN ISOLATION LEVEL REPEATABLE READ; SELECT 1; UPDATE memberships SET role = 'viewer' WHERE ...;` (it blocks)
- Commit A, then commit B, then `SELECT count(*) FROM run_recovery_namespaces WHERE subject = '<subject>';`. A result of 1 confirms it.

Relevant files:
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/data-model.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/plan.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/contracts/errors.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0136-failure-handlers-run-between-the-stop-and-the-terminal-commit.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/migrations/20261126000000_authority_read_locks.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/declared_operations.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/payload_disclosure.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/api-http/src/routes/execution.rs`
