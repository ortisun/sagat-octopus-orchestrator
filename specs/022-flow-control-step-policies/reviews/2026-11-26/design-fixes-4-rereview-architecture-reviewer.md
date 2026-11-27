# Architecture re-read of the third re-read fixes (2026-11-27, commits `12459df2d` and `ace26dc77`)

Short re-read (OD-2026-11-25-2 step 4) by the architecture reviewer, against
[design-fixes-3-rereview-architecture-reviewer.md](design-fixes-3-rereview-architecture-reviewer.md) and the "Third
re-reads (2026-11-27)" section of [design-fixes-responses.md](design-fixes-responses.md).

**Verdict.** Nothing here blocks, and I see no reason to hold back acceptance of the 2026-11-26 and 2026-11-27 amendments to ADR-0050, ADR-0056, ADR-0135, ADR-0136 and ADR-0137. One condition: ADR-0136's Consequences line (:211) still says the dispatch budget bounds a handler that starts fresh runs, while decision 5, as extended by D022-310, now says such starts are never dispatched. Correct that sentence in the change that records the acceptance. The new Major A is fixed: the lock function follows `authority_read_locks.up.sql`, the runtime role keeps SELECT, INSERT and DELETE only, and the premise "follows worker bindings" is corrected. Minors C to F, nits G to J and the earlier minors 5 and 8 are fixed in the design. Minor B is only partly fixed. Its ADR wording is right, but the check it promises cannot be written as specified: "deciding path" has no definition a script can use, and the code already has more step locks than the single allow-list entry. Nit K stays open on purpose. The security-driven changes keep the layering: the scope is in the domain `Principal`, and the locks, triggers and foreign key are in the adapter. They add no cycle to the slot order, and the ADR index matches every Status line. The fixes do introduce three new minors and some nits, listed below. I could not run `git branch -a` or `git worktree list` (no shell). The duplicate-work statement in design-fixes-responses.md:169-172 is the coordinating session's claim, and I did not verify it.

### Items of the third architecture report

| Item | Verdict | Evidence |
|---|---|---|
| A, Major: binding row not lockable by `sagat_app` | fixed | research.md:3169-3187 (D022-297); data-model.md:817, :879-880, :885-888; ADR-0050:91, :146-149; ADR-0136:99-101; plan.md:1107; tasks.md:1237, :1241, :1243, :1248. Premises hold: `authority_read_locks.up.sql:1-3`, `:4-10`, `:24-35`; `declared_operations.rs:236-238`. The slot-order reference runs on the owner's pool (`slot_order.rs:61-63` creates tables with it; `lock_order.rs:304`), so the reference can lock the row directly while the path runs as `sagat_app`. |
| B, minor: step-lock rule in neither ADR, not checkable | partly fixed | Wording fixed: ADR-0056:114-122, ADR-0135:57-62, research.md:3338-3347, tasks.md:1238. Premise holds: `prepare_delivery` (`scoped_events.rs:101`) encloses `:164`, and `bindings.rs:244-246` updates only `LEASED`/`RUNNING`. The check is still not specifiable; see "What remains" below. |
| C, minor: slice 2 needs parts of T022-256 | fixed | tasks.md:1234, :1239, :1247, :1253; research.md:3327-3337; openapi-flow-control.yaml:704-707; plan.md:1115. One leftover is in nit 7. |
| D, minor: ADR-0136 and ADR-0050 records lag | fixed | ADR-0136:3, :4, :6, :224-226; ADR-0050:6. The same gap remains in ADR-0056 and ADR-0135 (nit 6). |
| E, minor: `RunRecovery` starts not in the handler tree | fixed | ADR-0136:111-115, :161-165; data-model.md:890-894; errors.md:53; spec.md:659-662; plan.md:1117, :1190; tasks.md:1235. It conflicts with D022-243 (new finding 1). |
| F, minor: "every principal rebuilt from a stored member" unchecked | fixed | research.md:3289-3306; tasks.md:1240. I checked the site list against the tree and it is complete. Every other production `Principal::new(` passes a literal role: `worker.rs:1286-1291`, `:1632-1637`, `schedule_firing.rs:757`, `:1774-1778`, `batch_executor.rs:245`, `:267`, `loop_progress.rs:604`, `reset_completion.rs:431`, `:447`. `api_admission.rs:735` (under `#[cfg(test)]` at `:501`) and `pending_reset.rs:756` are test code. |
| G, nit: budget line | fixed | plan.md:1137 |
| H, nit: leftovers | fixed | spec.md:596-597, :629-631; ADR-0056:93-94; ADR-0050:3, :5, :217. Other leftovers are in nit 7. |
| I, nit: batch rerun sentence | fixed | errors.md:138. Premise holds at `batch_executor.rs:439-444`. The citation `identity.rs:514` is weak (nit 7). |
| J, nit: "after the memberships" | fixed | ADR-0050:91, :150-155; research.md:3381-3389. Premise holds at `slot_order.rs:696-697`. The slot-5 claim agrees with ADR-0135:46-51: those memberships are the publisher's and the producer's. |
| K, nit: no recorded acceptance for the 2026-11-26 revisions | open, as intended | design-fixes-responses.md:120-121; ADR-0050:146-147; ADR-0056:118-119; ADR-0135:3; ADR-0136:3 |
| Earlier minor 5 and ruling 7 | fixed | through A |
| Earlier minor 8 | partly fixed | through B |

**What remains of B.** The check of task T022-255 is not specifiable as written:

- **No scope a script can use.** The rule applies on a "deciding path", which no document defines in terms of files or functions.
- **More hits than the one allow-list entry.** Production code already has other step and session locks that the rule must either allow or exclude by name:
  - `crates/infra-postgres/src/execution/path.rs:733-735` (`charge_path_output_bytes`, `FROM steps ... FOR UPDATE`).
  - `crates/infra-postgres/src/primitives.rs:2781-2790` (`start_collection_drain`, `LEASED`/`RUNNING` steps `FOR UPDATE OF s`). This one matters after ADR-0146 routes collection finishes through the reducer in slice 3.
  - `crates/infra-postgres/src/execution.rs:994-996` (the claim's `worker_sessions ... FOR UPDATE`).
- **Lock forms missing.** The rule names only `FOR UPDATE`. A binding replacement's update also conflicts with `FOR SHARE`, `FOR NO KEY UPDATE` and the lock an `UPDATE steps` statement takes.
- **Smallest fix.**
  - Define the scope by enclosing function: every function that asks the lookup round, and the paths that call them.
  - Cover all of those lock forms.
  - List each current hit with its argument, or its exclusion by name, in D022-308 before slice 7.

### New findings

**1. minor: D022-310 contradicts D022-243, which is still in force, and three records still describe the old behaviour.**
- **The contradiction.** D022-243 (research.md:2645-2650) keeps the dispatch budget as the bound for a handler that starts fresh runs, triggers or Signal-with-Start. It explicitly rejected refusing fresh starts, because "a notifier that opens a ticket workflow is the main use". D022-310 (research.md:3365-3366) rejects the budget because "a budget defers, it never ends a loop", but that loop stays open through an event trigger or a Signal-with-Start the handler causes.
- **What is stale.**
  - ADR-0136:211 still says such runs are "bounded by the namespace dispatch budget".
  - plan.md:1196 still lists that row.
  - The evidence of task T022-239 (tasks.md:1124) asks for "a handler that starts fresh failing runs" to be deferred at the budget. With D022-310, a handler using its `RunRecovery` credential creates handler-tree runs that are never dispatched, so that test cannot be built as written.
  - The notifier's ticket workflow is now never dispatched when it fails, and is refused `422` if it continues as new. No record states this.
- **Fix.**
  - Amend D022-310 to say it narrows D022-243: the `RunRecovery` path no longer loops; triggers and Signal-with-Start remain bounded only by the budget.
  - Rewrite ADR-0136:211 as two Bad consequences: a recovery start's failure is not dispatched, and a definition that continues as new cannot be started by the role.
  - Drive T022-239's evidence with an event trigger, and correct plan.md:1196.

**2. minor: the scope refusal's conversion cannot queue the denial record, and its match has a wildcard arm.**
- **Where.** research.md:3258 (D022-301) and tasks.md:1239 queue the denial record "by the conversion".
- **Why it cannot work.**
  - `impl From<IdentityError> for AppError` (`crates/application/src/error.rs:360-369`) is a pure synchronous function with no context.
  - The denial path is the async method `record_cross_tenant(&self, .., ctx)` (`api_admission.rs:457`).
- **The wildcard.**
  - `error.rs:366` maps every unlisted variant through `other => Self::Invalid(other)`, which becomes a `422` carrying the error's text (`api-http/src/error.rs:990`).
  - So `IdentityError::NamespaceScopeRequired` (T022-263, contracted as `403` at errors.md:48), and `OutsideNamespaceScope` if its arm is forgotten, would answer a distinguishable `422`.
  - The event stream counts a `422` as a tick error and keeps going (`routes/execution.rs:1412-1415`). This defeats D022-301's own argument that "no match can forget a variant".
- **Fix.**
  - The resolvers that call `authorize_in` record the denial, then return `AppError::NotFound`; the conversion stays pure.
  - Add explicit arms in `error.rs` for both variants (`NamespaceScopeRequired` to `Forbidden { permission }`), in T022-263 and T022-256.
  - Make the match exhaustive, or add a unit test per `IdentityError` variant.

**3. minor: `RUN_RECOVERY_BINDINGS_EXIST` has no producer.**
- **The contract.** errors.md:70-73, plan.md:1108, T022-261 and T022-265 say "the provisioning command" reports it for a role change.
- **No such command exists.** Provisioning only inserts memberships (`crates/infra-postgres/src/identity.rs:146-155`, `:170-185`), and no production statement updates `memberships`. D022-298 itself says role changes are out of band.
- **Consequence.** The "one test per error" of T022-265 has nothing to call. Adding a role-change command would be new identity provisioning, which belongs outside feature 022, with an audit action that does not exist today.
- **Fix (smallest).** Drop the code. Out-of-band SQL sees `23503` naming `run_recovery_namespaces_membership_fk`. T022-261 asserts that SQLSTATE and constraint name, and the runbook step of T022-267 says what it means.

**4. nit: the database now writes audit vocabulary.** The cascade audit trigger (data-model.md:882, T022-266) would be the first SQL writer of `audit_records`; no migration inserts into it today. It writes `run_recovery.unbound`, `run_recovery_binding` and `system:provisioning` as string literals, while the closed vocabulary lives in `AuditAction`/`ResourceType` (`crates/domain/src/audit.rs:19`, pinned by `crates/domain/tests/audit.rs`). Fix: T022-266 asserts the trigger's row against `AuditAction::…as_str()`, `ResourceType::…as_str()` and `Subject::provisioning()`.

**5. nit: the `.permissions()` rule of T022-258 cannot see the receiver's type.** `.permissions()` is also called on an admission operation (`api_admission.rs:263`, `:274`) and on a pending-reset entry (`runs/pending_reset.rs:485`, `:628`). Fix: refuse every `.permissions()` outside the domain unless its enclosing function is listed, and list these four as non-`Role` entries.

**6. nit: "Enforced by" still lags in ADR-0056 and ADR-0135.** Their restated rules rely on T022-254 and T022-255, which are named inline only (ADR-0056:119-122, ADR-0135:61-62). ADR-0056:7-16 and ADR-0135:6 do not name them; the fix of D was applied to ADR-0050 and ADR-0136 only.

**7. nit: leftovers.**
- ADR-0050:217-220 still reads "None yet" and "Required before acceptance" in an accepted ADR.
- ADR-0050:139-145 (the 2026-11-26 paragraph, "reads … `FOR SHARE`") is not marked superseded, as ADR-0136:99-100 marks its own.
- tasks.md:1150 still gives T022-240 (slice 2) "the widened `memberships_role_valid` and OpenAPI role enum".
- plan.md:1084-1086 describes the old foreign key to `memberships` without D022-298's mark.
- D022-313 cites `identity.rs:514`, which is the `run:batch` string mapping; `RunRecovery` is not built yet (`Role` at `identity.rs:529-542`). Cite ADR-0136 decision 4 and the catalog test of T022-240 instead.

**8. nit: ADR-0136's Considered options omit the alternatives of decision 4's schema choices.** These are recorded only in D022-297 and D022-298: the role-change trigger that misses a concurrent bind at REPEATABLE READ, a refusal by isolation level, and a one-column UPDATE grant. Add one line each.

**Blocking.** Nothing blocks the ADR acceptance beyond the ADR-0136:211 sentence. B's rest blocks T022-255 in slice 7. New findings 2 and 3 must be fixed before slice 13. Finding 1 should be fixed in the acceptance change.

Main files:
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/contracts/errors.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0136-failure-handlers-run-between-the-stop-and-the-terminal-commit.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/error.rs
