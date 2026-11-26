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
