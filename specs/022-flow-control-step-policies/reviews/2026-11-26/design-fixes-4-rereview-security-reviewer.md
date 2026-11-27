# Security re-read of the third re-read fixes (2026-11-27, commits `12459df2d` and `ace26dc77`)

Short re-read (OD-2026-11-25-2 step 4) by the security reviewer, against
[design-fixes-3-rereview-security-reviewer.md](design-fixes-3-rereview-security-reviewer.md) and the "Third re-reads
(2026-11-27)" section of [design-fixes-responses.md](design-fixes-responses.md).

## Verdict

Commits `12459df2d` and `ace26dc77` handle every item of the third security re-read. Thirteen items are fixed and one is partly fixed (nit 3). None is left unfixed. The design uses the three hand-checked facts correctly:
- **Lock function.** It matches the `sagat_lock_memberships` precedent line for line.
- **Restricting foreign key.** It has a legal target: `memberships` has primary key `(tenant_id, subject)`, so `UNIQUE (tenant_id, subject, role)` can be referenced. Its checks run as the table owner, so `sagat_app` needs no UPDATE on `memberships`.
- **Cascade audit.** `sagat_app` cannot forge it, because it holds no DELETE on `memberships` or `namespaces`.

I found no Blocker and no Major. The fixes bring three new minors and four nits. All of them fail closed, so nothing grants access. The most serious minor: the scope refusal now depends on an `IdentityError` to `AppError` conversion that the compiler does not force. Today that conversion has a wildcard arm, so a missing arm silently becomes a validation error. The second minor: the denial audit has two triggers that never meet, so for the most common case (a run outside the scope) nothing is audited and nothing is counted.

Paths are relative to `/Users/marcelo/Documents/Projects/sagat-wt-022-policies/`.

## Items of the report

| Item | Verdict | Evidence |
|---|---|---|
| Earlier minor 1, residual (bindings survive a role change) | Fixed | D022-298 (`research.md:3188-3218`), `data-model.md:875-877`. The race is closed declaratively, as fact 3 shows. The primary key `(tenant_id, subject)` (`migrations/20261008000000_identity.up.sql:50`) makes the new unique key a legal target. |
| Earlier minor 2, remainder, and new finding 4 (`.permissions()`, keying by file) | Fixed, with nits 4 and 5 | D022-303 (`research.md:3281-3308`), T022-258 (`tasks.md:1240`). Premises hold: `identity.rs:618`, `declared_operations.rs:252`, `execution.rs:1369-1395` (`current_worker_authorized`), `routes/identity.rs:49-54`. The list of `Principal::new(` sites with a non-literal role is complete (checked by grep). Two entry names are wrong (nit 4). |
| Earlier minor 3 (pending reset completion) | Fixed | D022-301 (`research.md:3262-3268`). `PendingAdmission` has no namespace today (`runs/pending_reset.rs:138-146`); `PendingReset.namespace_id` exists (`:93`). `failure_of` maps `StateConflict(AUTHORITY_REVOKED)` (`reset_completion.rs:226-228`). `recheck_requester` is at `execution/resets.rs:655-681`. |
| Earlier minors 4, 5, 6; earlier nits 1, 3, 4 | Fixed, unchanged | As in the previous report. D022-300 restates the list of functions (`research.md:3243-3252`). |
| Earlier nit 2 (order of refusals) | Fixed | `contracts/errors.md:44`, `spec.md:596-597`, `plan.md:1114`. |
| Earlier suspicion (stream window) | Fixed in the design | The loop ends on `NotFound` (`routes/execution.rs:1408`, `:1437`). The 30 seconds hold only once the conversion arm exists (new minor 1). |
| New finding 1 (runtime role cannot lock the binding) | Fixed | `research.md:3169-3187`, `data-model.md:879`, T022-264 (`tasks.md:1248`). It mirrors `authority_read_locks.up.sql:4-10` and `:24-35`: `VOLATILE`, `FOR SHARE OF b`, revoked from PUBLIC, granted inside the guard. ADR-0050 (`:148`) and ADR-0136 (`:100-101`) agree. |
| New finding 2 (new `AppError` variant missed by the stream) | Fixed; the conversion is not pinned (new minor 1) | `research.md:3253-3269`. |
| New finding 3 (payload disclosure not fenced) | Fixed, and buildable | The disclosure transaction reads the namespace from the run row it locks (`infra-postgres/src/payload_disclosure.rs:86-107`), then takes the namespace and the membership (`:107-109`), so the binding can be taken right after. The lock order (run, namespace, membership, binding) does not invert a namespace deletion's (namespace, then binding by the cascade). |
| Nit 1 (role change deletes without audit) | Fixed, residual in new nit 6 | No trigger deletes any more; the cascade is audited (D022-299). |
| Nit 2 (`check_role_before_lookup` returning `()`) | Fixed | D022-304 (`research.md:3309-3311`). |
| Nit 3 (call-log test cannot see the decoded rows) | Partly fixed | The scoped lookup removes the decode (D022-305), but it removes the trigger of the denial record too (new minor 2). |
| Nit 4 (slice-2 task) | Fixed | T022-263 (`tasks.md:1247`). |
| Suspicion (REPEATABLE READ race) | Fixed | D022-298 relies on fact 3. Checked: `sagat_app` holds SELECT on `memberships` and SELECT, INSERT on `namespaces` only (`migrations/20261008000200_runtime_role_grants.up.sql:19-20`), so it cannot cause a cascade or forge a `system:provisioning` row. |

## New findings

### 1. minor: the mapping of both scope errors to HTTP answers is not pinned, and today a wildcard catches them

Verified in the code. `crates/application/src/error.rs:360-367`:
```rust
IdentityError::PermissionDenied { permission, .. } => Self::Forbidden { permission },
IdentityError::TenantMismatch => Self::NotFound,
other => Self::Invalid(other),
```
`AppError::Invalid` becomes a validation error whose body is `inner.to_string()` (`crates/api-http/src/error.rs:990`). Neither T022-263 (`NamespaceScopeRequired`, slice 2) nor T022-256 (`OutsideNamespaceScope`) names an arm here, and the compiler forces none.

**Scenario.** A `RunRecovery` principal hits a route whose resolver was forgotten. The answer is `422 VALIDATION_FAILED`, not the `403` the plan states (`plan.md:1062`). Three things then go wrong:
- The event stream counts the error as a tick error and continues (`routes/execution.rs:1412-1415`), keeping the stream open for 15 minutes.
- `failure_of` returns `None` for `Invalid` (`reset_completion.rs:256`), so a pending reset retries forever.
- The T022-257 check "never `403 NamespaceScopeRequired`" passes vacuously.

For `OutsideNamespaceScope`, a missing arm gives a validation body instead of the opaque `404`. For a namespace addressed by id or slug, B and a missing id are both refused against the bound set, so they get the same wrong answer and T022-257's equality check still passes. D022-301's argument ("no match can forget a variant that does not exist") moved the forgettable match into this wildcard. No data leaks.

**Fix.** Add explicit arms `NamespaceScopeRequired { permission } => Forbidden { permission }` and `OutsideNamespaceScope => NotFound`. Remove the wildcard, or add a unit test in `crates/application/src/error.rs` pinning both, as evidence of T022-263 and T022-256. T022-257 should also assert the opaque `404` status for namespace B, not only equality with a missing id.

### 2. minor: the denial record is queued "by the conversion", but the scoped lookup never produces the error that is converted

Verified in the design. D022-301 (`research.md:3258`, `plan.md:1110`) queues `run_recovery.namespace_denied` when `OutsideNamespaceScope` is converted. D022-305 (`research.md:3312-3320`, `data-model.md:896-899`) puts `AND namespace_id = ANY($bound)` in the run, workflow and request lookups. A hit outside the scope then returns zero rows and the resolver answers a plain `NotFound`, so `OutsideNamespaceScope` is never raised for them. A `From` implementation also has no handle on the denial task.

**Scenario.** A subject bound to namespace A reads run R in namespace B. Zero rows come back, the answer is `404`, no audit row is written and `run_recovery_namespace_denials_total` does not move. D022-275 is lost for the most common case. T022-256's application tests use a store double that returns the row, so they still see the audit. Its PostgreSQL rows test checks only that zero rows come back.

**Fix.** State the trigger once: for a bound principal, every empty scoped lookup, and every bound-set refusal of a namespace, hands the identifier to the bounded denial task. That task runs the unscoped existence query off the response path and writes the record only when the resource exists in the tenant outside the scope. Keep the conversion pure. The PostgreSQL rows test of T022-256 should assert one audit row for a hit outside the scope and none for a miss.

### 3. minor: one existing `NotFound` handler reads the scope refusal as "done"

Verified in the code. `crates/application/src/schedule_firing.rs:1240` treats `Err(AppError::NotFound)` from `cancel_run` as "the run is gone" and records the cancellation as paid (`:1247-1251`); only `Forbidden` revokes (`:1242-1243`).

**Scenario.**
1. Subject X, then a Developer, creates a schedule in namespace B with overlap `CANCEL_OTHER`.
2. An operator changes X's role to `run-recovery`. Nothing refuses this: D022-298 refuses a role change only while bindings exist.
3. At the next firing, `find_membership` and the role checks for `run:start` and `run:operate` pass (`:764-768`), since the role holds both. `Principal::new(.., membership.role())` (`:774`) then gives an empty scope.
4. The owed cancel passes the role check, the scoped lookup returns no row, and the debt is recorded paid with the run still running.
5. Every later start is refused and propagated as an error (`:1332`). The schedule is wedged, never `revoke_authority`.

The backfill has the same shape (`:454-462`). No privilege is gained, but the overlap guarantee breaks silently. This disproves D022-301's claim that every existing `NotFound` handler then behaves correctly.

**Fix.** In the firing and backfill paths, treat a namespace-bound authority as revoked up front (`!m.role().is_namespace_bound() && ...` at `:764` and `:456`). That is also the argument the `.grants(` and `Principal::new(` allow-list entries of `ScheduleFiring::fire` and `ScheduleFiring::backfill_step` need. Add a T022-256 case for it.

### 4. nit: two allow-list entries name functions that do not exist under D022-303's own keying

- The `evaluate` at `crates/application/src/payload_disclosure.rs:619` is in `impl DisclosureEvaluator for ServiceEvaluator` (`:600`). Line `:311` is the trait's declaration (`trait DisclosureEvaluator`, `:303`), not a method of `DisclosureAudits`.
- `PgDeclaredOperationStore::authorize` is really the free function `authorize` at `crates/infra-postgres/src/declared_operations.rs:218`, outside the `impl` at `:35`.
- `ServiceEvaluator::evaluate` also serves the schedule input disclosure (`crates/application/src/schedule_disclosure.rs:202-210`, `read: ScheduleRead`), whose transaction takes no binding (`infra-postgres/src/payload_disclosure.rs:395-410`).

This fails closed: the rule would refuse the real sites. Fix: name the entries `ServiceEvaluator::evaluate` (with two arguments: the binding of D022-302, and `schedule:read`, which the role does not hold) and `declared_operations::authorize`.

### 5. nit: a text rule cannot tell when `.permissions()` is called on a `Role`

Other production calls with the same name: `operation.permissions()` (`crates/application/src/api_admission.rs:263`, `:274`) and `entry.permissions()` (`crates/application/src/runs/pending_reset.rs:485`, `:628`). The script will either flag all four or miss a `Role` call made through a variable. Fix: rename `Role::permissions` to a name no other type uses and refuse that name, keeping `auth_context` as its one entry. Alternatively, allow-list the four sites.

### 6. nit: the cascade audit misses a direct delete done out of band, and the runbook makes removal slower than it needs to be

D022-299 audits a delete only when a parent row is gone. Suppose an operator who meets the role-change refusal and has no tenant administrator token runs `DELETE FROM run_recovery_namespaces ...` as the owner. Both parent rows exist, so the trigger writes nothing. That contradicts "no removal is audited twice or not at all" (`research.md:3230-3231`).

Fix: the route calls `set_config('sagat.unbind_audited','on',true)` before its delete, and the trigger audits every delete unless that setting is on and both parents exist. Add a direct-delete case to T022-266.

Separately, T022-267 (`tasks.md:1251`) has "removing the member" start by unbinding, but a membership deletion already cascades and is audited (`contracts/errors.md:73-75`). The runbook should say that removing a member (the incident-response action) needs no unbind first.

### 7. nit: `RUN_RECOVERY_BINDINGS_EXIST` belongs to a provisioning command that does not exist, and the two foreign keys both raise SQLSTATE `23503`

Provisioning only inserts memberships (`crates/infra-postgres/src/identity.rs:142-155`). No production statement updates or deletes `memberships`; the one delete is in `20261014000000_scripts.down.sql:39`. The operator therefore sees PostgreSQL's raw foreign-key error, not a code that names the namespaces still bound. T022-265's "one test per error" has nothing to call.

The bind route has the same ambiguity: the namespace key also raises `23503` on the bind's insert. Fix: map `23503` by constraint name (`run_recovery_namespaces_membership_fk` gives `NOT_A_RECOVERY_SUBJECT`, the namespace key gives the opaque `404`). Either add a role-change provisioning command that maps it, or drop the code and document the raw error in T022-267.

## Suspicion (outside this feature, not run)

The precedent D022-286 copied, `memberships_role_change_revokes_reveal_grants` (`migrations/20261220000000_reveal_grants_role_change.up.sql:11-27`), is the same kind of `AFTER UPDATE OF role` delete trigger that fact 2 showed racing at REPEATABLE READ. If `grant_reveal` locks the grantee's membership `FOR SHARE` and inserts while a REPEATABLE READ role change waits, the grant survives the demotion. To check, repeat fact 2's two `psql` sessions with `payload_reveal_grants` in place of the binding table. If it holds, it is a gap for feature 016; the same restricting foreign key would close it.

## Classes with no finding (looked)

- **Injection and parsing:** `$bound` is a bound array parameter, and the new functions use no dynamic SQL.
- **Security-definer functions:** all three pin `search_path`, qualify every table with its schema and are revoked from PUBLIC.
- **Secrets in the new audit row:** its reason is a fixed string and its identifier is the one the route writes.
- **Lock-order cycles:** none among an unbind, a membership deletion, the payload disclosure and slot 4a.

## Relevant files
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/error.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/schedule_firing.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/payload_disclosure.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/schedule_disclosure.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/payload_disclosure.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/declared_operations.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/migrations/20261008000200_runtime_role_grants.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/migrations/20261220000000_reveal_grants_role_change.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/data-model.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/contracts/errors.md
