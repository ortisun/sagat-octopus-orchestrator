# Responses to the re-reads of the third-read fixes (2026-11-26)

Where each finding of
[design-fixes-rereview-security-reviewer.md](design-fixes-rereview-security-reviewer.md) and
[design-fixes-rereview-architecture-reviewer.md](design-fixes-rereview-architecture-reviewer.md) now lives. Each
decision is in `research.md` ("Decisions of the re-reads of 2026-11-26"), delegate (OD-2026-11-24-4). Every security
finding is fixed in the design; none is accepted or deferred. "Plan" is `plan.md`, section "Release 0.3.0, the rest of
022", subsection "Revision of 2026-11-26 after the re-reads" unless a row says otherwise; "data model" is
`data-model.md`, section "Revision of 2026-11-26 after the re-reads"; "tasks" is `tasks.md`, subsection "Revision of
2026-11-26 after the re-reads". The code facts were checked on this branch at `1b21a101`.

## Security re-read

| Finding | Decision | Where it lives |
|---|---|---|
| Major 1, part 1 (scope bound to named namespaces; not where it is enforced, nor that no row means no namespace): partly fixed | D022-272 (the enforcement, see new finding 1) | as new finding 1 |
| Major 1, part 2 (who grants the binding, its audit): partly fixed | D022-274 (see new finding 3) | as new finding 3 |
| Major 1, part 3 (what the role can and cannot do): fixed | no change; the table gains the binding's routes (D022-274) | plan, "Authorization (T022-011)", the `RunRecovery` table |
| Major 1, part 4 (payloads, secrets, failure text): fixed, no access | no change | — |
| Major 1, part 5 (widening by a machine identity): fixed, cannot | no change | — |
| Major 2 (the role may not redrive a dispatch): fixed | no change; the order of its refusals in new finding 5 | — |
| Major 2, related (role not assignable before `failure-handler-v1` is activated): partly fixed | D022-276 (see nit 6) | as nit 6 |
| Security item A (machine-identity leftovers): fixed, one nit | D022-283 (see nit 7) | as nit 7 |
| New finding 1, Major: the binding is a rule written per route and fails open where a route forgets it | D022-272, D022-275 (the reviewer's facts verified: `crates/domain/src/identity.rs:742-785`, `crates/application/src/api_admission.rs:274-324`, `:332`, `crates/application/src/runs.rs:973-984`, `crates/api-http/src/blobs.rs:118-122`) | ADR-0136 decision 4 (revised 2026-11-26) and "Enforced by"; plan, "The namespace scope of a principal" and the `RunRecovery` table; spec FR-022-014 and FR-022-032 lines after the re-reads; `contracts/errors.md` (the revised `404` row and the fail-closed `403`); tasks T022-256 (the principal's scope, the resolvers), T022-257 (the battery of every route as a `RunRecovery` subject against an unbound namespace), T022-258 (the rule of `scripts/check-architecture.py`), T022-240 as it reads now |
| New finding 2, minor: the new table's grants contradict their precedent | D022-273 | data model, "`run_recovery_namespaces`, revised" (grants SELECT, INSERT, DELETE; composite foreign keys to `namespaces` and `memberships`, `ON DELETE CASCADE`; the subject CHECK; the quota triggers); the line of the third reads marked superseded; tasks T022-253 as it reads now |
| New finding 3, minor: the grant surface is not specified | D022-274 | `contracts/openapi-flow-control.yaml` (`listRunRecoveryBindings`, `bindRunRecovery`, `unbindRunRecovery`, `RunRecoveryBinding`, `MembershipRoleAdditions`); `contracts/errors.md` (`NOT_A_RECOVERY_SUBJECT`, the activation, `404` and `403` rows); plan, the `RunRecovery` table and the audit actions under "Observability, added"; ADR-0136 decision 4; `contracts/README.md`; tasks T022-253 as it reads now |
| New finding 4, minor: an attempt outside the binding is not audited | D022-275 | ADR-0136 decision 4; plan, "Observability, added" (`run_recovery.namespace_denied`, `run_recovery_namespace_denials_total`); `contracts/errors.md` (the revised `404` row); tasks T022-256 |
| New finding 5, nit: the redrive does not say the `404` outside the binding comes before `403 RECOVERY_ROLE_MAY_NOT_REDRIVE` | D022-272 (the scope is checked before the role) | `contracts/openapi-flow-control.yaml` (the redrive's description); `contracts/errors.md` (the revised `404` row); plan, the `RunRecovery` table; ADR-0136 decision 4 |
| New finding 6, nit: the activation gate has no error row and no rule for switching the capability off | D022-276 | `contracts/errors.md` (the provisioning paragraph and the bind's `FLOW_CONTROL_ACTIVATION_PENDING` row); data model (the `memberships` trigger, the empty bound set); plan, failure matrix (the capability switched off); spec FR-022-014 line after the re-reads; ADR-0136 decision 4; tasks T022-253 as it reads now |
| New finding 7, nit: task T022-228's evidence still names a machine identity | D022-283 | `tasks.md`, T022-228 (struck and replaced by a `RunRecovery` credential) |
| Suspicion: a replay after a revocation (is the binding read on every request, does a replay check it again) | D022-277; checked, the suspicion does not hold once D022-272 is built: every request is authenticated again (`crates/api-http/src/extract.rs:52`, `:88`) and reads the membership with no cache (`crates/infra-postgres/src/identity.rs:362-377`); the start route admits before the use case (`crates/api-http/src/routes/execution.rs:474-480`) and `start_with` resolves its namespace (`crates/application/src/runs.rs:1391`) before every receipt lookup (`:1424`, `:1699`, `:1768`). The window between authentication and the write is closed by the slot-4a read | ADR-0136 decision 4; ADR-0050 decision 7 (slot 4a and the paragraph written 2026-11-26); plan, failure matrix (a revocation before the write, a replay after a revocation) and "The namespace scope of a principal"; spec FR-022-014 and FR-022-032 lines; `contracts/errors.md` (the revoked-binding row); tasks T022-259 |

## Architecture re-read

| Finding | Decision | Where it lives |
|---|---|---|
| Ruling 1 (dispatch outcome in `run.finished`): partly met | D022-280 (see minor 3) | as minor 3 |
| Ruling 2 (one draining cause, a further stop refused `409`): met | no change | — |
| Ruling 3 (the overdue pass between pick and lock): met, D022-262 said it differently | D022-278 (see Major 1) | as Major 1 |
| Ruling 4 (continue-as-new refused at admission): partly met | D022-279 (see Major 2) | as Major 2 |
| Ruling 5 (a budget deferral is not an attempt): met | no change | — |
| Ruling 6 (the producer's locks of a hit): met; the slot table lags | D022-281 (see minor 7) | as minor 7 |
| Ruling 7 (`RunRecovery`): partly met | D022-277, D022-273 (see minors 5 and 6) | as minors 5 and 6 |
| Ruling 8 (slices that must merge alone): partly met | D022-283 (see minor 9) | as minor 9 |
| Ruling 9 (ADR-0146 order): met | no change | — |
| Ruling 10 (the draft-executions lock): met | no change; written into the slot table (D022-281) | ADR-0050 decision 7, slot 4a |
| Ruling 11 (leftovers): partly met | D022-283 (see minor 10) | as minor 10 |
| Acceptance (D022-271): met | no change; the "Decided by" lines (see the last nit) | — |
| Major 1: the overdue no-op is tied to the wrong thing | D022-278, restating D022-262 (the reviewer's location of `StopReason` is wrong, see the note below; the conclusion holds) | ADR-0136 decision 2; spec FR-022-013 line after the re-reads; plan, failure matrix row of the overdue pass and "The overdue pass"; research D022-262 marked restated; tasks T022-243 as it reads now (the domain case: the deadline a no-op, the handler timer draining) |
| Major 2: continue-as-new under a handler tree is not refused for resets and reruns | D022-279 | ADR-0136 decisions 5 and 9; ADR-0137 decision 5 (the defence's code and status); data model (`runs.handler_tree` row and "`runs.handler_tree`, restated"); `contracts/errors.md` (the reset and rerun `422` row; the durable code on a run `FAILED` and on a batch item); spec FR-022-016 line after the re-reads; plan, failure matrix and "Continue-as-new under a handler tree"; tasks T022-244 as it reads now |
| Minor 3: three places still call the dispatch statements "last" | D022-280 | ADR-0136 decision 7; ADR-0050 decision 7 (the lineage extension); plan, "The order of a commit"; data model, "Lock order, as built" and "The dispatch statements and invariant 22" |
| Minor 4: invariant 22 compares against the row's state "at the commit" | D022-280 | data model (invariant 22 restated, the old line marked); tasks T022-238 as it reads now |
| Minor 5: the binding check has no lock | D022-277, D022-281 | ADR-0050 decision 7 (slot 4a, and the paragraph written 2026-11-26); ADR-0136 decision 4; data model (readers of `run_recovery_namespaces`); tasks T022-259 (the `slot_order.rs` case) |
| Minor 6: `run_recovery_namespaces` grants SELECT only yet is written by a route that runs as `sagat_app` | D022-273 | as security new finding 2 |
| Minor 7: the ADR-0050 slot table lags the amendments | D022-281 | ADR-0050 decision 7, slots 4a, 5 and 7 |
| Minor 8: task T022-255 is too broad; T022-254 and T022-255 have no slice | D022-282 (the reviewer's facts verified: `crates/infra-postgres/src/execution/scoped_events.rs:164`, `crates/infra-postgres/src/bindings.rs:245-246`) | tasks T022-254 and T022-255 as they read now (slice 7, with T022-210) |
| Minor 9: older slice bullets not struck | D022-283 | `tasks.md`, the evidence bullets of slices 2 and 3 in the revisions of 2026-11-25 (struck) |
| Minor 10: leftovers outside ruling 11 | D022-283 | data model (the dispatch note and the configuration note, struck and replaced); `tasks.md`, T022-214 and its row of 2026-11-25, T022-227 (struck); plan, the hot-path line of a failing run with a default handler (struck) |
| Minor 11: the overdue "skipped" count has no closed label | D022-278 | plan, "Observability, added" (`run_deadline_outcomes_total`, `outcome` ∈ enforced, skipped_handler_phase); tasks T022-243 as it reads now |
| Minor 12: nothing checks the ADR index against the status lines | D022-284 (the reviewer's facts verified: `scripts/check-agent-config.py:13`, `:215`) | tasks T022-260 (a check of `scripts/check-agent-config.py`, described, not built here) |
| Nit: ADR-0136 decision 8 counts before it checks the budget, T9 the reverse | D022-285 | ADR-0136 decision 8 |
| Nit: ADR-0094 still calls ADR-0146 proposed | D022-285 | ADR-0094 decision 4's revision of 2026-11-25 |
| Nit: nested parentheses in ADR-0056 decision 2 | D022-285 | ADR-0056 decision 2 |
| Nit: the plan's "the first cause wins" | D022-285 | plan, failure matrix (one cause) |
| Nit: the plan's `RunRecovery` "column" is a separate table | D022-285 | plan, "Authorization (T022-011)" (named a table of its own) |
| Nit: the stored value of the role is not named | D022-285 (`run-recovery`, after `crates/domain/src/identity.rs:731` and migration `20261124000000_database_executor.up.sql:4`) | ADR-0136 decision 4; data model; `contracts/openapi-flow-control.yaml` (`MembershipRoleAdditions`); tasks T022-240 as it reads now |
| Nit: the "Decided by" lines of ADR-0135, ADR-0136, ADR-0137, ADR-0146 still read "accepted once ... a re-review" | D022-285 | the four "Decided by" lines; their status lines note the 2026-11-26 revision, the first word still `Accepted` |

## Reviewer claims found wrong in the code

- Architecture Major 1 cites `StopReason` at `crates/domain/src/execution/command.rs:2521-2529` with three variants
  (`Cancelled`, `TimedOut`, `Failed`). Those lines are `stop_run`; `StopReason` is at
  `crates/domain/src/execution/event.rs:369` and has six variants (also `Compensated`, `Partial`, `CompensationFailed`).
  The conclusion holds: the deadline and the handler's timer share the one reason `TimedOut`.
- Security new finding 4 cites `record_cross_tenant` at `crates/application/src/api_admission.rs:368-375`: those lines
  are its call in `resolve_workflow`; the function itself is at `:457-480`. The point holds.
- Every code claim the fixes rely on was checked and holds (the citations in the table above). Of the evidence for
  findings judged fixed, a sample was checked and holds (`crates/domain/src/identity.rs:313-315`,
  `crates/api-http/src/core.rs:35`, `crates/infra-postgres/src/execution.rs:5726-5735`,
  `crates/domain/src/execution/command.rs:2483`); the rest was not checked again.

## Second security re-read (of `9960a3c1`)

Where each item of [design-fixes-2-rereview-security-reviewer.md](design-fixes-2-rereview-security-reviewer.md) now
lives. Decisions in `research.md`, "Decisions of the second security re-read of 2026-11-26", delegate
(OD-2026-11-24-4); every item fixed, none accepted or deferred. "Tasks" here is `tasks.md`, subsection "Revision of
2026-11-26 after the second security re-read"; "data model" is its subsection of the same name. Code facts checked at
`ddbac141`; every code claim of the record was checked and holds (see the note below on `:1276`).

| Finding | Decision | Where it lives |
|---|---|---|
| Minor 1: a role change leaves the bindings in place | D022-286 | ADR-0136 decision 4 (revision of the second re-read); data model (the role-change trigger, the bind under `sagat_lock_memberships`); `contracts/errors.md` (the bind's `422` after a concurrent role change); plan, failure matrix (a role changed and changed back) and the scope table; tasks T022-261, T022-253 as it reads now |
| Minor 2: the fitness rule matches four permission names; `Membership::restore` is public; the role-only method is not named | D022-287 | tasks T022-258 as it reads now (every `.grants(` judged by enclosing function, the allow list of the tree's sites; `check_role_before_lookup` callers; `Membership::restore(` confined), T022-256 as it reads now; plan, the scope table ("The role-only check") |
| Minor 3: the pending reset completion is not covered | D022-288 | ADR-0136 decision 4; ADR-0050 decision 7 (the paragraph of 2026-11-26, the completion's re-check in slot 4a); data model; `contracts/errors.md` (pending resets, `AUTHORITY_REVOKED`); plan, failure matrix and the scope table; spec FR-022-014 line after the second re-read; tasks T022-262 |
| Minor 4: the battery accepts "`403` or the opaque `404`" | D022-289 | tasks T022-257 as it reads now (status and body equal to those of a missing identifier); `contracts/errors.md` (the revised `404` row) |
| Minor 5: the activation trigger's function attributes are unspecified | D022-290 | data model (both trigger functions: `SECURITY DEFINER`, pinned search path, schema-qualified names, `REVOKE ALL ... FROM PUBLIC`, `ENABLE ALWAYS`); ADR-0136 decision 4; tasks T022-253 as it reads now, T022-261 |
| Minor 6: merge order could open a window | D022-291 (both the reviewer's fix and a stronger one: the role is namespace-bound with an empty scope from slice 2) | tasks T022-011 and T022-240, T022-249, T022-253 as they read now and the slice sentence; data model (the migration lands with T022-256); plan, the scope table ("From slice 2"); ADR-0136 decision 4 |
| Nit 1: T022-256 does not pin the scope class of the role's permissions | D022-292 | tasks T022-256 as it reads now (the domain test) |
| Nit 2: the revised `404` row says "before any other refusal of the route" | D022-293 | `contracts/errors.md` (the row reworded: after the role check, before every other refusal) |
| Nit 3: the denial audit is written only when the resource exists, so time tells existing slugs | D022-294 | data model (the bound set loaded with slugs); plan, the scope table ("A miss and a denial"); `contracts/errors.md` (the revised `404` row); tasks T022-256 as it reads now |
| Nit 4: `tasks.md` still says "machine identity" in T022-228's reversed amendment | D022-295 | `tasks.md`, T022-228's amendment of 2026-11-25 struck whole |
| Suspicion: the event stream's maximum lifetime sets the revocation window | D022-296: the maximum lifetime is 15 minutes (`crates/api-http/src/streams.rs:49`), but the window is the 30-second re-check (`streams.rs:50`; `crates/api-http/src/routes/execution.rs:1385-1400`), once `RunService::get` checks the scope | research D022-277 (completed); plan, "The namespace scope of a principal" and the failure matrix; spec FR-022-014 line; tasks T022-259 and T022-256 as they read now |

Note on the suspicion's citation: `crates/api-http/src/routes/execution.rs:1276` is not stale; it is the call of
`stream_deadline`, whose body is at `:1304`. Both are cited now.

## Third re-reads (2026-11-27)

Where each item of [design-fixes-3-rereview-security-reviewer.md](design-fixes-3-rereview-security-reviewer.md) (of
`5ef574f16`) and [design-fixes-3-rereview-architecture-reviewer.md](design-fixes-3-rereview-architecture-reviewer.md)
(of `9960a3c1` and `5ef574f16`) now lives. Decisions in `research.md`, "Decisions of the third re-reads of 2026-11-27",
each delegate (OD-2026-11-24-4); every security item is fixed in the design, none accepted or deferred. "Plan" is
`plan.md`, subsection "Revision of 2026-11-27 after the third re-reads" unless a row says otherwise; "data model" is
`data-model.md`, section "Revision of 2026-11-27 after the third re-reads"; "tasks" is `tasks.md`, subsection "Revision
of 2026-11-27 after the third re-reads". Code facts checked at `5ef574f16`. Three facts were checked by hand on
`postgres:16-alpine` by the coordinating session and are cited in the decisions as facts 1 to 3: a row lock needs
UPDATE privilege; the race of the security suspicion is real at REPEATABLE READ; a restricting foreign key closes it at
every isolation level.

Acceptance of the revisions of 2026-11-26 and 2026-11-27 to the accepted ADRs (ADR-0050, ADR-0056, ADR-0135, ADR-0136,
ADR-0137) is recorded after the short re-read of these fixes (OD-2026-11-25-2 step 4), not here (architecture nit K).

### Security

| Finding | Decision | Where it lives |
|---|---|---|
| Earlier minor 1 (a role change leaves the bindings in place): fixed, residual in nit 2 and the suspicion | D022-298 (the trigger replaced, see the suspicion) | as the suspicion |
| Earlier minor 2 (the fitness rule, `Membership::restore`, the role-only method): partly fixed, `.permissions().contains(` unseen and the allow list keyed by file | D022-303 | tasks T022-258 as it reads now; research D022-287 (amended) |
| Earlier minor 3 (the pending reset completion): partly fixed | D022-297 (the lock), D022-301 (the error and `PendingAdmission.namespace_id`) | tasks T022-262 as it reads now; plan, the scope table of 2026-11-26 (marked) and the table of 2026-11-27; data model (the readers row); research D022-288 (amended); `contracts/errors.md` (pending resets) |
| Earlier minor 4 (the battery's "403 or 404"): fixed | no change | — |
| Earlier minor 5 (the activation trigger's function attributes): fixed | no change; the list of functions restated by D022-300 | data model, the functions paragraph |
| Earlier minor 6 (merge order): fixed, residual in nit 4 | D022-307 (see nit 4) | as nit 4 |
| Earlier nit 1 (the scope class of the role's permissions): fixed | no change; its test moves to T022-263 (D022-307) | tasks T022-263 |
| Earlier nit 2 ("before every other refusal"): partly fixed | D022-306 | `contracts/errors.md` (the revised `404` row); `spec.md` FR-022-014 line after the re-reads (reworded); plan, the table of 2026-11-27 |
| Earlier nit 3 (time tells existing slugs): fixed, residual in nit 3 | D022-305 (see nit 3) | as nit 3 |
| Earlier nit 4 ("machine identity" in T022-228): fixed | no change | — |
| Earlier suspicion (the stream lifetime sets the window): partly fixed, it depends on new finding 2 | D022-301 | as new finding 2; research D022-296 (completed) |
| New finding 1, minor: the runtime role cannot take the binding row lock | D022-297 (fact 1; the false premise "follows worker bindings" corrected, `20261010000100_worker_bindings.up.sql:25`) | data model (the lock function and readers rows); research D022-273, D022-277, D022-288 (amended); ADR-0050 decision 7 (slot 4a and the paragraph written 2026-11-27); ADR-0136 decision 4 (revision of 2026-11-27); plan, the tables of 2026-11-26 (marked) and 2026-11-27; tasks T022-264, T022-253, T022-259 and T022-262 as they read now (run as `sagat_app`) |
| New finding 2, minor: a new `AppError` variant misses the stream's `NotFound` arms | D022-301 (no new variant: `IdentityError::OutsideNamespaceScope` converts to `AppError::NotFound`; `complete_reset` checks the scope first and answers `StateConflict(AUTHORITY_REVOKED)`; `PendingAdmission` gains `namespace_id`) | plan, the table of 2026-11-26 (marked) and of 2026-11-27; research D022-288 (amended), D022-296 (completed); `contracts/errors.md` (pending resets); ADR-0136 decision 4; tasks T022-256, T022-259 (the stream case) and T022-262 as they read now |
| New finding 3, minor: the allow list admits payload disclosure, which is not fenced against a revocation | D022-302 | `contracts/errors.md` (the payload row); plan, failure matrix and the table of 2026-11-27; data model (the readers row); ADR-0136 decision 4; tasks T022-259 as it reads now (the payload case) |
| New finding 4 (counted under earlier minor 2) | D022-303 | as earlier minor 2 |
| Nit 1: a role change deletes bindings without an audit row | D022-298 (no trigger deletes: the role change is refused until the administrator unbinds through the audited route), D022-299 (a removal by a cascade of a membership or namespace deletion audited by a trigger; checked: the platform has no deletion path of its own, `crates/infra-postgres/src/identity.rs:141-168`) | data model (the audit trigger); `contracts/errors.md` (the provisioning paragraph of 2026-11-27); plan, failure matrix; tasks T022-266, T022-261 as it reads now |
| Nit 2: `check_role_before_lookup` "returning `()`" | D022-304 | plan, the scope table of 2026-11-26 (corrected); research D022-287 (amended); tasks T022-256 as it reads now |
| Nit 3: a call-log test cannot see the decoded rows | D022-305 (`AND namespace_id = ANY($bound)` in the lookups of a bound principal; the call-log test becomes a rows test) | data model ("The scoped lookups of a bound principal"); plan, the table of 2026-11-27; research D022-294 (refined); tasks T022-256 as it reads now |
| Nit 4: which slice-2 task carries the empty scope | D022-307 | tasks T022-263 (slice 2), T022-011, T022-240 and T022-256 as they read now; research D022-291 (amended) |
| Suspicion: the bind and role-change race reopens at REPEATABLE READ | D022-298, superseding D022-286; confirmed by fact 2, closed by the restricting foreign key of fact 3 at READ COMMITTED, REPEATABLE READ and SERIALIZABLE; the provisioning error `RUN_RECOVERY_BINDINGS_EXIST` and the runbook step (T022-267) | data model (`run_recovery_namespaces` and `memberships`, revised; the trigger of 2026-11-26 struck); research D022-286 (superseded), D022-290, D022-291 (amended); `contracts/errors.md` (the bind's `422` row, the provisioning paragraph); `contracts/openapi-flow-control.yaml` (`bindRunRecovery`, `unbindRunRecovery`); ADR-0136 decision 4; ADR-0050 decision 7; spec FR-022-014 line of 2026-11-27; plan, failure matrix (the old row struck, three rows added) and the table of 2026-11-27; tasks T022-261 (rewritten), T022-253, T022-265, T022-267 |

### Architecture

| Finding | Decision | Where it lives |
|---|---|---|
| Major 1, Major 2, minors 3, 4, 6, 7, 9, 10, 11, 12, the earlier nits, rulings 1, 3, 4, 6, 8, 11: fixed | no change | — |
| Minor 5 (the binding check has no lock): partly fixed | D022-297 (see A) | as A |
| Minor 8 (T022-255 too broad): partly fixed | D022-308 (see B) | as B |
| Ruling 7 (`RunRecovery`): partly fixed | D022-297 (see A) | as A |
| Acceptance (index against status lines): holds | no change; every changed ADR keeps `Accepted` as the first word of its Status line | — |
| A, Major: the binding row cannot be read `FOR SHARE` by the runtime role | D022-297 | as security new finding 1; the `pg_proc` test of T022-253 reads the lock function (D022-300) |
| B, minor: the narrowed step-lock rule is in neither ADR and cannot be checked from the SQL text | D022-308 | ADR-0056 decision 2(a) and ADR-0135 decision 2 (restated in D022-282's words); research D022-282 (amended); plan, the table of 2026-11-27; tasks T022-255 as it reads now (an allow list of enclosing functions, one entry `prepare_delivery`, a test that the row is `WAITING`) |
| C, minor: the empty scope of slice 2 needs parts of T022-256 | D022-307 | tasks T022-263, T022-256 as it reads now (the OpenAPI role enum in its pull request); `contracts/openapi-flow-control.yaml` (`MembershipRoleAdditions`); plan, the table of 2026-11-27 |
| D, minor: ADR-0136's record lags; ADR-0050 names no feature 022 `slot_order.rs` case | D022-309 | ADR-0136 Status, "Decided by", "Enforced by" and Evidence; ADR-0050 "Enforced by" (T022-224, T022-254, T022-259, T022-262) |
| E, minor: a start by a `RunRecovery` principal is not a handler-tree run | D022-310 | ADR-0136 decisions 5 and 9; data model (`runs.handler_tree` row and "extended"); `contracts/errors.md` (the continue-as-new row); spec FR-022-016 line of 2026-11-27; plan, the `RunRecovery` table, failure matrix and the table of 2026-11-27; tasks T022-244 as it reads now |
| F, minor: "every principal rebuilt from a stored member" has no check | D022-303 (`Principal::new(` with a non-literal role on the same allow-list rule; the sentence narrowed to what the rule checks) | research D022-272 (narrowed), D022-287 (amended); tasks T022-258 as it reads now |
| G, nit: the membership statement also reads `platform_requirements` | D022-311 | plan, "Hot-path budget lines to record" (the member's request line) |
| H, nit: leftovers (spec "before any other refusal", "the revision read with the run's definition", ADR-0050 calling ADR-0056 proposed) | D022-312, D022-306 | `spec.md` FR-022-014 and FR-022-016 lines (struck and replaced); ADR-0056 decision 2; ADR-0050 Status, "Refines / supersedes" and Evidence |
| I, nit: the batch rerun case needs one sentence | D022-313 (`RunRecovery` holds no `run:batch`, `crates/domain/src/identity.rs:514`; a batch item runs under its creator's re-read role, `crates/application/src/batch_executor.rs:440`) | `contracts/errors.md` (the batch item row) |
| J, nit: "after the memberships" assumes a lock a start does not take | D022-314 (the foreign key of D022-298 replaces the trigger the reviewer named: it refuses a role change while the row exists, so the binding row fences an unbind, a role change and a membership removal) | ADR-0050 decision 7 (slot 4a and the paragraph written 2026-11-27); research D022-277 (amended); data model (the readers row); plan, the scope table of 2026-11-26 (marked) |
| K, nit: the 2026-11-26 amendments have no recorded acceptance | none: the acceptance of the revisions of 2026-11-26 and 2026-11-27 is recorded after the short re-read of these fixes (OD-2026-11-25-2 step 4) | the paragraph above; the amendments of 2026-11-27 say so where they are written (ADR-0050 decision 7, ADR-0056 decision 2(a), the Status lines of ADR-0135 and ADR-0136) |

Note on the architecture reviewer's scope check, which it could not run: on this branch's machine at `5ef574f16`,
`git branch -a` and `git worktree list` show the 022 branches `feature/022-step-policies` (this one),
`feature/022-throughput-control` and `feature/022-sdk-builders` in sibling worktrees, and neither of the other two (nor
`origin/develop`) has `RunRecovery` or `run_recovery` code under `crates/`, so no duplicate of this scope was found.

## Fourth re-reads (2026-11-27)

Where each item of [design-fixes-4-rereview-security-reviewer.md](design-fixes-4-rereview-security-reviewer.md) and
[design-fixes-4-rereview-architecture-reviewer.md](design-fixes-4-rereview-architecture-reviewer.md) (both of
`12459df2d` and `ace26dc77`) now lives. Decisions in `research.md`, "Decisions of the fourth re-reads of 2026-11-27",
each delegate (OD-2026-11-24-4) and following the coordinating session's rulings of this round; every finding is fixed
in the design, none accepted or deferred. "Plan" is `plan.md`, subsection "Revision of 2026-11-27 after the fourth
re-reads" unless a row says otherwise; "data model" is `data-model.md`, section "Revision of 2026-11-27 after the fourth
re-reads"; "tasks" is `tasks.md`, subsection "Revision of 2026-11-27 after the fourth re-reads". Code facts checked at
`ace26dc77`.

Acceptance of the revisions of 2026-11-26 and 2026-11-27 to the accepted ADRs follows the short re-read of this round
(architecture nit K and the verdict's condition); it is not recorded here.

### Security

| Finding | Decision | Where it lives |
|---|---|---|
| Earlier items: thirteen fixed, nit 3 partly fixed (the scoped lookup removed the trigger of the denial record) | no change for the thirteen; nit 3's rest is new minor 2 | as new minor 2 |
| New minor 1: the mapping of both scope errors to HTTP answers is not pinned, and a wildcard arm catches them | D022-315 (an exhaustive conversion with no wildcard arm, every existing variant mapped to what the wildcard gives it today, a unit test per variant; T022-257 asserts the opaque `404` status for an unbound namespace) | tasks T022-268 (new), T022-263, T022-256 and T022-257 as they read now; plan, the table and the failure matrix; `contracts/errors.md` (the `403` row); research D022-301 (marked); ADR-0136 decision 4 (revision of the fourth re-reads) |
| New minor 2: the denial record is queued by the conversion, which the scoped lookup never reaches | D022-316 (the conversion stays pure; every empty scoped lookup and every bound-set refusal hands the identifier to the bounded denial task, which writes the record only for a resource that exists in the tenant outside the scope) | tasks T022-269 (new), T022-256 as it reads now (one audit row for a hit, none for a miss); data model ("The denial record of a bound principal"); plan, the table; `contracts/errors.md` (the revised `404` row); research D022-275, D022-294, D022-301, D022-302, D022-305 (marked) |
| New minor 3: `ScheduleFiring::pay_obligation` reads the scope refusal as "done" | D022-317 (a namespace-bound authority revoked before any cancel or start: `revoke_authority` in `fire`, the backfill's revocation branch `stop_backfill` in `backfill_step`; the argument of the entries `ScheduleFiring::fire` and `ScheduleFiring::backfill_step`; D022-301's claim corrected to name `pay_obligation`) | tasks T022-270 (new), T022-256 and T022-258 as they read now; plan, the table and the failure matrix; research D022-301 and D022-303 (marked) |
| Nit 4: two allow-list entries name functions that do not exist | D022-318 (`ServiceEvaluator::evaluate` with two arguments, the binding of D022-302 and `schedule:read`; `declared_operations::authorize`, a free function) | tasks T022-258 as it reads now; research D022-302 and D022-303 (marked); tasks, the T022-258 row of the third re-reads (marked) |
| Nit 5: a text rule cannot tell when `.permissions()` is called on a `Role` | D022-319 (`Role::permissions` renamed `Role::granted_permissions` in T022-263; T022-258 refuses that name outside `crates/domain`, one entry `identity::auth_context`) | tasks T022-263 and T022-258 as they read now; plan, the table and the fitness-rule row of the third re-reads (marked); research D022-303 (marked) |
| Nit 6: a direct delete out of band is not audited; the runbook slows the removal of a member | D022-320 (the unbind route sets `sagat.unbind_audited`; the trigger audits every other delete), D022-321 (removing a member needs no unbind first) | data model (the marker and the audit trigger, revised); tasks T022-266 (a direct-delete case) and T022-267 as they read now; `contracts/errors.md` (the provisioning paragraph of 2026-11-27); plan, the failure matrix; research D022-298 and D022-299 (marked) |
| Nit 7: `RUN_RECOVERY_BINDINGS_EXIST` has no producer, and both foreign keys raise `23503` | D022-322 (the code dropped everywhere; an out-of-band role change sees SQLSTATE `23503` naming `run_recovery_namespaces_membership_fk`), D022-323 (the bind maps `23503` by constraint name: the membership key to `NOT_A_RECOVERY_SUBJECT`, `run_recovery_namespaces_namespace_fk` to the opaque `404`) | tasks T022-261, T022-265 and T022-267 as they read now; data model (the keys, revised); `contracts/errors.md` (the provisioning paragraph, a new `404` row); `contracts/openapi-flow-control.yaml` (`bindRunRecovery`); plan, the tables and the failure matrix; research D022-298 (marked) |
| Suspicion (outside this feature): the reveal-grant role-change trigger races at REPEATABLE READ | none in feature 022: confirmed by design reading, and fixed on its own bugfix branch outside feature 022; nothing about reveal grants changes here | — |

### Architecture

| Finding | Decision | Where it lives |
|---|---|---|
| Verdict's condition: ADR-0136's Consequences line still says the dispatch budget bounds a handler that starts fresh runs | D022-328 | as new finding 1 |
| A, C to J, earlier minor 5 and ruling 7: fixed | no change | — |
| B, minor, its rest (earlier minor 8): the check of T022-255 is not specifiable (no scope, more hits, lock forms missing) | D022-324 (a deciding path is a transaction that runs `Engine::run` or `Engine::run_addressed`; their callers named; a second check refuses a new caller), D022-325 (`FOR UPDATE`, `FOR NO KEY UPDATE`, `FOR SHARE`, `FOR KEY SHARE` and `UPDATE` of `steps` and `worker_sessions`), D022-326 (the 44 current statements, each argued `after-rounds`, `not-leased` or `no-round`, or excluded by name; `charge_path_output_bytes`, `start_collection_drain`, the claim's `worker_session_room` and `prepare_delivery` among them) | tasks T022-255, T022-207 as they read now, T022-271 (new); research D022-308 (marked); ADR-0056 decision 2(a); ADR-0135 decision 2; plan, the table and the step-lock row of the third re-reads (marked) |
| K, nit: acceptance not recorded | none: acceptance follows the short re-read of this round | the paragraph above |
| New 1, minor: D022-310 contradicts D022-243, and three records still describe the old behaviour | D022-327 (D022-310 narrows D022-243: the `RunRecovery` path no longer loops; triggers and Signal-with-Start stay bounded by the budget only), D022-328 (ADR-0136's Consequences as two Bad consequences; T022-239 driven by an event trigger; the plan's row corrected) | ADR-0136 decision 5 and Consequences; research D022-243 and D022-310 (marked); spec FR-022-016 line of the fourth re-reads; plan, the failure matrix (the row reworded) and the `RunRecovery` starts row of the third re-reads (marked); tasks T022-239 (evidence amended) |
| New 2, minor: the conversion cannot queue the denial record, and its match has a wildcard arm | D022-315, D022-316 | as security new minors 1 and 2 |
| New 3, minor: `RUN_RECOVERY_BINDINGS_EXIST` has no producer | D022-322 | as security nit 7 |
| Nit 4: the database now writes audit vocabulary | D022-329 (T022-266 asserts the trigger's row against `AuditAction`, `ResourceType` and `Subject::provisioning()`) | tasks T022-266 as it reads now; data model (the audit trigger, revised) |
| Nit 5: the `.permissions()` rule cannot see the receiver's type | D022-319 | as security nit 5 |
| Nit 6: "Enforced by" lags in ADR-0056 and ADR-0135 | D022-330 (both name T022-254, T022-255 and T022-271) | ADR-0056 and ADR-0135 "Enforced by" |
| Nit 7: leftovers (ADR-0050 Evidence and its 2026-11-26 paragraph, T022-240's line, the plan's old key, D022-313's citation) | D022-331 | ADR-0050 decision 7 and Evidence; tasks, the T022-240 row of the third rulings (marked); plan, "The binding's table and routes" (marked); research D022-313 (corrected) |
| Nit 8: ADR-0136's Considered options omit decision 4's schema alternatives | D022-332 | ADR-0136 Considered options 10 to 12 |

Note on the architecture reviewer's scope check, which it could not run: at `ace26dc77`, `git branch -a` and `git
worktree list` show the 022 branches `feature/022-step-policies` (this one), `feature/022-throughput-control` and
`feature/022-sdk-builders`, each in its own worktree; this round changes documents of this branch only.

Numbers: decisions D022-315 to D022-332 and tasks T022-268 to T022-271 are used; D022-333 to D022-340 and T022-272 to
T022-280 are left free. *(Used by the fifth re-reads below: D022-333 to D022-349 and T022-272.)*

## Fifth re-reads (2026-11-27)

Where each item of [design-fixes-5-rereview-security-reviewer.md](design-fixes-5-rereview-security-reviewer.md) and
[design-fixes-5-rereview-architecture-reviewer.md](design-fixes-5-rereview-architecture-reviewer.md) (both of
`1b2547d5e`) now lives. Decisions in `research.md`, "Decisions of the fifth re-reads of 2026-11-27", each delegate
(OD-2026-11-24-4) and following the coordinating session's rulings of this round; every finding is fixed in the design,
none accepted or deferred. "Plan" is `plan.md`, subsection "Revision of 2026-11-27 after the fifth re-reads" unless a
row says otherwise; "data model" is `data-model.md`, section "Revision of 2026-11-27 after the fifth re-reads"; "tasks"
is `tasks.md`, subsection "Revision of 2026-11-27 after the fifth re-reads". Code facts checked at `1b2547d5e`.

Acceptance of the revisions of 2026-11-26 and 2026-11-27 to the accepted ADRs follows a narrow re-read of the fix of
architecture new finding 1 (the root claim's lock order) and security new minor 1 (the start transaction's authority
check); it is not recorded here.

### Security

| Finding | Decision | Where it lives |
|---|---|---|
| Items of the fourth report: all fixed, minor 3 with the lock it relies on missing | no change, but minor 3's rest is new minor 1 | as new minor 1 |
| New minor 1: the namespace-bound check is not inside the transaction that fences the start | D022-344 (`recheck_authority` admits only `role.grants(Permission::RunStart) && !role.is_namespace_bound() && ...`; the existing `AUTHORITY_REVOKED` branches answer for a firing, a backfill step and a trigger-now; the argument of the `schedules::recheck_authority` allow-list entry) | tasks T022-270 (three held-lock cases) and T022-258 as they read now; plan, the table and the failure matrix; ADR-0136 decision 4 and "Enforced by" (revision of the fifth re-reads); research D022-317 (marked) |
| New minor 2: a bound principal's cross-tenant attempt is no longer specified | D022-345 (the resolver skips the inline `*_exists_outside` for a bound principal; the task checks existence without the scope and tenant predicates: `run_recovery.namespace_denied` in the tenant, `CrossTenantAccessDenied` in another tenant, nothing for a miss) | tasks T022-269 as it reads now (an other-tenant case); data model ("The denial record of a bound principal, revised"); plan, the table and the failure matrix; ADR-0136 decision 4; research D022-316 and the data model's section of the fourth re-reads (marked) |
| Nit 3: a hand-off dropped under load is neither audited nor counted | D022-346 (the hand-off takes a slot of the same `denial_slots`; a drop increments `audit_denials_total{outcome="dropped"}`; the task writes through `insert_denial_once`) | tasks T022-269 as it reads now (a case with every slot held); data model (the same section); plan, the failure matrix |
| Nit 4: the unbind marker departs from its precedent and is never cleared | D022-347 (the marker is the binding's key, set immediately before the route's delete and cleared immediately after; the trigger skips only a deleted row whose key equals it and whose parents both exist; the plain grants of D022-273 kept, the definer function rejected) | tasks T022-266 as it reads now (two deletes in one transaction); data model ("`run_recovery_namespaces`: the unbind marker, revised"); plan, the table, the failure matrix and the row of the fourth re-reads (marked); research D022-320 (marked) |
| Nit 5: the `.granted_permissions(` rule exempts more than the `.grants(` rule | D022-348 (exempt only `crates/domain/src/identity.rs`) | tasks T022-258 as it reads now; plan, the table and the row of the fourth re-reads (marked); research D022-319 (marked) |

### Architecture

| Finding | Decision | Where it lives |
|---|---|---|
| Acceptance condition: two text edits to ADR-0136 and ADR-0135 | D022-349 (ADR-0135 decision 2 lists the four argument kinds, `key-held` among them; ADR-0136 decision 5 and its Consequences strike "or a Signal-with-Start") | ADR-0135 decision 2, Status and "Enforced by"; ADR-0136 decision 5, Consequences, Status and "Enforced by"; ADR-0056 decision 2(a) and "Enforced by" |
| Items of the fourth report: fixed, B's lock forms and current statements partly fixed, new 1 partly fixed | the rests are new findings 1 to 5 | as those rows |
| New 1, Major: the root claim holds the worker's session row and can then run a lookup round | D022-333 (`claim_root_run` takes the claimer's binding key shared through `RunTx::hold_worker_binding` before `worker_session_room`, the scoped claim's order; task T022-272 in its own pull request, merged alone before slice 7, T022-210 not merging before it), D022-334 (the fourth argument kind `key-held`; an entry of kind `no-round` or `key-held` reached through a `RunTx` method names the method's application callers), D022-335 (T022-271's lease test replaced by a `slot_order.rs` case of a root claim refusing an input whose `onError` successor is cacheable with an entry the claimer produced, against a replacement of the claimer's binding, both orders) | tasks T022-272 (new), T022-210, T022-255 and T022-271 as they read now; plan, the table, the hot-path line and the failure matrix; data model ("Lock order of a root claim"); ADR-0135 decision 2; ADR-0056 decision 2(a); research D022-326 (the `worker_session_room` entry marked) |
| New 2, minor: `PgRunTx::database_owner` argued from wrong premises; its red step cannot turn red | D022-336 (argued `key-held` by the lock at `crates/infra-postgres/src/execution/database.rs:38-47`; the red step removes that lock; the premises on `bindings.rs:91-93` and the protocol minor 6 corrected) | tasks T022-271 and T022-255 as they read now; plan, the failure matrix; research D022-326 (the `database_owner` entry marked) |
| New 3, minor: `after-rounds` is false in transactions that run the engine twice | D022-337 (`after-rounds` reworded; `Scheduler::decide_grant`, `RunService::finish_start` and, found by the same reading, `WorkerService::claim_root_run` listed with their arguments; check (2) refuses a function reaching two engine entry points unless listed) | tasks T022-255 as it reads now; plan, the table; ADR-0135 decision 2; research D022-324 (marked) |
| New 4, minor: the lock forms miss `INSERT ... ON CONFLICT DO UPDATE` and `DELETE` | D022-338 (both forms added; the registration, the replacement's delete and the expiry purge listed `no-round`; 47 statements; negative fixtures) | tasks T022-255 as it reads now; plan, the table; ADR-0056 decision 2(a); research D022-325 and D022-326 (marked) |
| New 5, minor: the Signal-with-Start clause contradicts D022-310, and T022-239 cannot be built | D022-341 (struck in ADR-0136 and research; a Signal-with-Start made with the `RunRecovery` credential is a recovery start; who publishes the event of an ordinary run), D022-342 (T022-239's event published by the test with its own `event:publish` credential; the slice evidence line corrected) | ADR-0136 decision 5 and Consequences; research D022-243, D022-310, D022-327 and D022-328 (marked); spec FR-022-016, the line of the fourth re-reads (marked); plan, the rows of the third and fourth re-reads and the failure matrix (marked); tasks T022-239 (its line, the row of the fourth re-reads and the row as it reads now) and the slices 10 to 13 evidence line (marked) |
| Nit 6: the allow list cannot be keyed by function alone | D022-339 (keyed by function and a `// step-lock: <name>` marker, a constant by its name; the claim's lock is the constant `RUN_FOR_CLAIM`) | tasks T022-255 as it reads now; plan, the table; research D022-326 (marked) |
| Nit 7: check (2) as worded has gaps | D022-340 (matched by the transaction argument, whatever the receiver; `run_addressed` included as a precaution) | tasks T022-255 as it reads now; plan, the table; ADR-0056 decision 2(a); research D022-324 (marked) |
| Nit 8: the unbind marker is transaction-wide | D022-347 | as security nit 4 |
| Nit 9: D022-317's backfill filter citation | D022-343 (`schedule_firing.rs:459`) | research D022-317 and tasks T022-270 (marked) |

Note on the architecture reviewer's scope check, which it could not run: at `1b2547d5e`, `git branch -a` and `git
worktree list` show the 022 branches `feature/022-step-policies` (this one), `feature/022-throughput-control` and
`feature/022-sdk-builders`, each in its own worktree; this round changes documents of this branch only.

Numbers: decisions D022-333 to D022-349 and task T022-272 are used; D022-350 and T022-273 to T022-280 are left free.
