# Architecture re-read of the re-read fixes (2026-11-27, commits `9960a3c1` and `5ef574f16`)

Short re-read (OD-2026-11-25-2 step 4) by the architecture reviewer, against
[design-fixes-rereview-architecture-reviewer.md](design-fixes-rereview-architecture-reviewer.md) and the "Architecture
re-read" section of [design-fixes-responses.md](design-fixes-responses.md); the changes of the second security re-read
judged for architecture only.

**Verdict.** No Blocker. Both Majors of the architecture report are fixed. I checked their premises against the code: `StopReason` is at `crates/domain/src/execution/event.rs:369-379` with six variants, and the overdue pass calls `stop_run(.., TimedOut, ..)` at `crates/scheduler/src/passes.rs:6684`. `refuse_gated_successor` returns `StoreError::Internal` at `crates/infra-postgres/src/chain_progress.rs:229-238`, and the `CHAIN_LIMIT` path is `passes.rs:1513-1529`. Minors 3, 4, 6, 7, 9, 10, 11 and 12 and every nit are fixed. Minors 5 and 8 are only partly fixed. Minor 5 now has its lock and its slot, but the lock cannot be taken under the grants the same fix chose; that is new Major A, the one finding to fix before slice 13. Minor 8 narrowed the task but not the two accepted ADRs that state the rule, and the narrowed rule cannot be checked from the SQL text. The changes of the second security re-read keep the layering: the scope lives in the domain `Principal`, and locks and triggers stay in the adapter. They add no cycle to the slot order. The ADR index matches every status line. The fixes do bring in one Major, five minors and some nits, listed below. None blocks the ADRs. I could not run `git branch -a` or `git worktree list` (no shell), so the duplicate-work check of scope was not done.

### Items of the architecture report

| Item | Verdict | Evidence |
|---|---|---|
| Major 1: the overdue no-op tied to a stop reason | fixed | ADR-0136:51-57; research.md:2799-2804 (D022-262 marked restated), :2956-2969; spec.md:570-572; plan.md:1088-1090, :1156; tasks.md:1178 |
| Major 2: continue-as-new under a handler tree for resets and reruns | fixed | ADR-0136:100-103, :149-152; ADR-0137:72-77 (the defence's code and status); errors.md:52, :128-129; data-model.md:666, :828-832; spec.md:646-649; plan.md:1030, :1092-1095, :1158; tasks.md:1179 |
| minor 3: "last" for the dispatch statements | fixed | ADR-0136:125-126; ADR-0050:128-129; data-model.md:779-780, :836-837; plan.md:934-935, :961 |
| minor 4: invariant 22 "at the commit" | fixed | data-model.md:801-802 (marked), :839-841; tasks.md:1180; spec.md:650 |
| minor 5: the binding check has no lock | partly | Lock, slot and test written: ADR-0050:91, :139-145; data-model.md:819, :855-856; tasks.md:1190, :1218. But `FOR SHARE` cannot be taken as `sagat_app` under the grants of minor 6 (new Major A) |
| minor 6: SELECT-only grant | fixed as asked | data-model.md:796-797 (marked), :817; research.md:2905-2913. The chosen grant set is what causes Major A |
| minor 7: the slot table lags | fixed | ADR-0050:91 (binding row, draft-executions key), :92 (pre-read, producer membership), :94 (re-check) |
| minor 8: T022-255 too broad, no slice | partly | tasks.md:1182-1183; research.md:2992-2997. I checked the premise: `scoped_events.rs:164` locks a step `FOR UPDATE` without a status predicate, and `bindings.rs:244-246` updates only `LEASED`/`RUNNING` rows. But ADR-0056:112-115 and ADR-0135:57-59 still state the broad rule, and the narrowed rule is not statically checkable (new finding B) |
| minor 9: older slice bullets | fixed | tasks.md:1046, :1107-1110 |
| minor 10: leftovers | fixed | data-model.md:683-685, :689-691; tasks.md:963, :1037, :1071; plan.md:1108. Other leftovers are in nit H |
| minor 11: the "skipped" label | fixed | plan.md:1136; tasks.md:1178 |
| minor 12: index against status lines | fixed in the design, not built | tasks.md:1191, :1194-1195 (slice 2); premise holds, `scripts/check-agent-config.py:13`, `:215` |
| Nits (ADR-0136 decision 8, ADR-0094, ADR-0056 parentheses, "first cause wins", the table, the stored role value, "Decided by") | fixed | ADR-0136:132-135; ADR-0094:44; ADR-0056:56-59; plan.md:1170, :1021-1023; ADR-0136:76; data-model.md:823; ADR-0135/0137/0146 line 4. The range on ADR-0136:4 is nit D |
| Rulings 1, 3, 4, 8, 11 (their open parts) | fixed | through minors 3 and 4, Major 1, Major 2, minor 9 and minor 10 above |
| Ruling 6 (the slot table) | fixed | through minor 7 above |
| Ruling 7 (`RunRecovery`) | partly | through minor 5 above (Major A) |
| Acceptance (index against status lines) | holds | docs/adr/README.md:99, :105, :142, :144-147 match the first Status word of each ADR |

The responses file's correction of the earlier reviewer is right: `StopReason` is in `event.rs`, not `command.rs`, and has six variants.

### New findings

**A. Major (not blocking the ADRs; fix before slice 13): the binding row cannot be read `FOR SHARE` by the runtime role.**
- **What the design says.** data-model.md:817 grants `run_recovery_namespaces` SELECT, INSERT and DELETE only. data-model.md:819 and :855-856, ADR-0050:91 and :141-145, and D022-277/D022-288 then read the row `FOR SHARE` as `sagat_app`, in slot 4a and in `recheck_requester`.
- **Why it fails.** PostgreSQL needs UPDATE privilege to take a row lock. The project already records this in `crates/infra-postgres/migrations/20261126000000_authority_read_locks.up.sql:1-3` (the reason `sagat_lock_memberships` exists) and in `crates/infra-postgres/src/approval_progress.rs:258-260`.
- **The premise is wrong too.** "Follows worker bindings" is not true: `20261010000100_worker_bindings.up.sql:25` grants UPDATE. data-model.md:756 also uses a one-column UPDATE grant for exactly this reason.
- **Consequence.** Built as written, every write by a `RunRecovery` subject and every completion of its pending resets fails with a permission error. It fails closed, but the role becomes unusable. The revocation fence of D022-277 and D022-288 does not exist as designed.
- **What will catch it.** `integration_platform_foundation::runtime_privileges` (architecture-principles.md:17, :53) and the "both database roles" run of T022-259 will fail. They catch it at build time, not at design time.
- **Fix.** Add a lock function, as 017 does for memberships:
  - `public.sagat_lock_run_recovery_binding(p_tenant uuid, p_subject text, p_namespace uuid)`, `SECURITY DEFINER`, `SET search_path = pg_catalog, pg_temp`, `FOR SHARE OF` the row;
  - `REVOKE ALL ... FROM PUBLIC`, `GRANT EXECUTE` to `sagat_app`;
  - called in slot 4a and in `recheck_requester`.

  Keep the plain INSERT and DELETE grants for the routes. D022-273 rejected a definer function only for the writer, not for the read lock. Add the function to the `pg_proc` attribute test of T022-253 and amend D022-273, D022-277, D022-288 and the data model's "Readers" row.

**B. minor: the narrowed step-lock rule is stated in neither accepted ADR and cannot be checked from the SQL text.**
- **The ADRs.** ADR-0056:112-115 and ADR-0135:57-59 still say "no decider locks a step or a `worker_sessions` row before its lookup round". The code breaks that today at `scoped_events.rs:164`.
- **The rule's checkability.** T022-255 (tasks.md:1183) now depends on the row's runtime status (`LEASED` or `RUNNING`). The statement at `:164` has no status predicate, so `scripts/check-architecture.py` cannot tell it apart from a forbidden lock.
- **Fix.** Restate both ADR sentences in D022-282's words. Write T022-255 as a refusal of every `steps ... FOR UPDATE` and `worker_sessions` lock on a deciding path, unless its enclosing function is on an allow list with its argument, as T022-258 does. Its one entry is the scoped-event finish ("a `WAITING` step; a binding replacement updates only `LEASED`/`RUNNING`"). Add a test that asserts the step is `WAITING` there.

**C. minor: the empty scope from slice 2 needs parts of T022-256, which lands in slice 13.**
- **Where.** tasks.md:1207 (D022-291) needs slice 2 to refuse every namespace-scoped permission to a `RunRecovery` principal. That requires `Role::is_namespace_bound`, the exhaustive scope class of `Permission` and the refusal in `Principal::authorize`. All three are items of T022-256 (tasks.md:1187), which lands in slice 13 (tasks.md:1193, :1220).
- **Why it matters.** It breaks the "each slice merges alone" rule of ruling 8: slice 2 would build half of a slice-13 task with no task of its own.
- **Also unclear.** It is not said whether the OpenAPI role enum (D022-266, "in the same change") moves with the migration.
- **Fix.** Split out a slice-2 domain task: the scope type, the scope class, the refusal, and its domain test (also D022-292's test). T022-256 keeps the bound set, the resolvers and the audit. State that the role enum lands with the migration in the pull request of T022-256.

**D. minor: ADR-0136's record lags the second security re-read.**
- **"Decided by".** ADR-0136:4 ends at D022-285 while the Status line (:3) runs to D022-296.
- **"Enforced by".** ADR-0136:6 omits:
  - T022-261 (the role-change trigger and the concurrent bind, both orders);
  - T022-262 (the pending reset, with its `slot_order.rs` case);
  - the stream case of T022-259;
  - the `pg_proc` attribute tests of T022-253 and T022-261;
  - the manifest test of the scope's artefacts (T022-249).
- **Evidence.** :211 and :213 list only T022-256 to T022-259 and D022-272 to D022-280.
- **ADR-0050.** Its "Enforced by" (ADR-0050:6) names none of the feature 022 `slot_order.rs` cases (T022-224, T022-254, T022-259, T022-262); they appear only inline.
- **Fix.** Extend those lines. This is the fitness rule: every promise names its check.

**E. minor: the principal criterion of D022-279 covers resets and reruns but not starts.**
- **Where.** ADR-0136:100-103 and data-model.md:830 make a reset or rerun by a `RunRecovery` principal a handler-tree run. A start by the same principal (it holds `run:start`) is not one.
- **Consequence.** Such a start may continue as new, and its failure is dispatched again. A handler that recovers by a fresh start therefore loops, limited only by the dispatch budget's rate (ADR-0136:198). A handler that recovers by a rerun cannot loop.
- **Fix.** Either include starts by a `RunRecovery` principal in `handler_tree`, with the continue-as-new refusal and one more case in T022-244, or state in ADR-0136 decision 5 why starts are excluded and that the budget is their only bound.

**F. minor: "every principal rebuilt from a stored member goes through `Membership::principal()`" has no check.**
- **Where.** research.md:2898-2900 and plan.md:1065 promise it. T022-258 (tasks.md:1212) confines only `Membership::restore(`.
- **The other sites.** `Principal::new` with a stored role is also called at:
  - `crates/application/src/batch_executor.rs:464`
  - `crates/application/src/schedule_firing.rs:546`, `:586`, `:774`
  - `crates/infra-postgres/src/approval_progress.rs:272`
- **Risk.** For `RunRecovery` these fail closed (empty scope), so there is no hole. The promise is just broader than its check.
- **Fix.** Add `Principal::new(` with a non-literal role to T022-258's allow list by enclosing function, or narrow the sentence to the reset completion.

**G. nit: the per-request membership statement now also reads `platform_requirements`.** data-model.md:819 (empty bound set while the capability is off) adds this read, but the budget line at plan.md:1115 names only the bound set. Name the extra read in that line.

**H. nit: leftovers.**
- spec.md:596 still says "before any other refusal", which contradicts errors.md:44 (D022-293).
- spec.md:622-623 and ADR-0056:93 still say "the revision read with the run's definition".
- ADR-0050:3 and :5 still call ADR-0056 "proposed", and ADR-0050:207 still says "this is a proposal".

**I. nit: the batch rerun case needs one sentence.** errors.md:129 and T022-244's batch case cover only a handler-tree source. Say why the principal case cannot happen: `RunRecovery` holds no `run:batch`.

**J. nit: "after the memberships" assumes a lock a start does not take.** ADR-0050:91 and :141-142 place the binding read after the memberships. A person's start takes no membership lock (`tests/fault/schedules-bulk-operations/slot_order.rs:696-697`), so for a `RunRecovery` start the binding row is the only authority lock. Say "after the memberships, where the path takes them". Also add that the role-change trigger's delete of that row fences a role change too.

**K. nit: the 2026-11-26 amendments have no recorded acceptance.** ADR-0050:139-145 and ADR-0136:79-97 add a new lock and new triggers to accepted ADRs "under the acceptance of D022-271". D022-271 accepted only the amendments of 2026-11-24 and 2026-11-25. Record an acceptance decision for the 2026-11-26 revisions after this re-read (OD-2026-11-25-2, step 4).

Files read:
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/reviews/2026-11-26/design-fixes-responses.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/data-model.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/plan.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/contracts/errors.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0056-concurrency-keys-are-durable-permits-granted-by-the-waiters-shard-owner.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0135-a-step-cache-hit-is-a-journaled-completion-decided-before-the-gates.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0136-failure-handlers-run-between-the-stop-and-the-terminal-commit.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0137-standalone-and-draft-test-runs-are-ordinary-runs-of-private-definitions.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/migrations/20261126000000_authority_read_locks.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/migrations/20261010000100_worker_bindings.up.sql
