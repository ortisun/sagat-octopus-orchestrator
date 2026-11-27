# Architecture re-read of the fourth re-read fixes (2026-11-27, commit `1b2547d5e`)

Short re-read (OD-2026-11-25-2 step 4) by the architecture reviewer, against
[design-fixes-4-rereview-architecture-reviewer.md](design-fixes-4-rereview-architecture-reviewer.md) and the "Fourth
re-reads (2026-11-27)" section of [design-fixes-responses.md](design-fixes-responses.md).

**Verdict.** The ADR condition from the last round is fixed. ADR-0136 now has the two Bad consequences, and the old sentence about the dispatch budget is struck (ADR-0136:230-238). New minors 2 and 3 and nits 4 to 8 are fixed. Minor B is only partly fixed. I checked both lists the implementer flagged against the code, and both are accurate. The callers list has 32 functions and 42 call sites, with nothing missing and nothing extra (the "37" figure appears in no document). The lock list has exactly 44 statements: 20 locking SELECTs and 24 UPDATEs. Some arguments behind the list are wrong, though, and one of them hides a real deadlock. The root claim locks the worker's session row, then in the same transaction can fail a step through `reject_input`, which runs `Engine::run`. Once the lookup round exists, that can wait on the binding key of the same worker identity. The `no-round` argument and the T022-271 test both miss this. The argument for the database executor's session lock reaches the right conclusion for the wrong reason, so the "shown red by hand" step T022-271 plans cannot be produced. New finding 1 of the last round is only partly fixed: the new Signal-with-Start clause contradicts D022-310 in the same ADR. I could not run `git branch -a` or `git worktree list` (no shell), so the claim at design-fixes-responses.md:219-221 is unverified.

**Acceptance and build.** The 2026-11-26 and 2026-11-27 amendments to ADR-0050, ADR-0056, ADR-0135, ADR-0136 and ADR-0137 can be accepted now, provided the acceptance change makes two text edits:
- Remove or qualify "or a Signal-with-Start" in ADR-0136:122-125 and :237-238 (new finding 5).
- In ADR-0135:63-64, either make the list of argument kinds open-ended or add "the row's subject's binding key held shared first" (new finding 1).

The build can start slice by slice up to slice 6, and slice 2 is not affected. Slice 7 (T022-210, T022-255, T022-271) is blocked by new findings 1 to 4. T022-239 and T022-244 (slices 10 to 13) need new finding 5 fixed first.

### Items of the fourth architecture report

| Item | Verdict | Evidence |
|---|---|---|
| Verdict's condition (ADR-0136:211) | fixed | ADR-0136:230-238; plan.md:1222 |
| B, rest: scope of the step-lock rule | fixed | research.md:3572-3604. Every production call matches the list. In `crates/application/src` there are 30 call sites in 20 functions. In `crates/scheduler/src` there are 12 call sites in 12 functions: events.rs:730; passes.rs:1514, :1585, :4844, :5083, :5186, :5303, :6262, :6376; collection_controls.rs:174; collection_timers.rs:145; collection_event_waits.rs:211. No production SQL on `steps` or `worker_sessions` exists outside `crates/infra-postgres/src`. The premise that `Engine::run_addressed` calls `Engine::resolve` is false (nit 7). |
| B, rest: lock forms | partly fixed | research.md:3605-3613 covers the four row-lock forms and UPDATE. It misses `INSERT ... ON CONFLICT DO UPDATE` and `DELETE` (new finding 4). |
| B, rest: current statements | partly fixed | The count is exactly 44: 20 locking SELECTs plus 24 UPDATEs, matching research.md:3619-3676 entry by entry. The `worker_session_room` argument is wrong (new finding 1). The `database_owner` premise is wrong (new finding 2). `after-rounds` does not hold in transactions that run the engine twice (new finding 3). The claim lock is misattributed and entries are keyed too coarsely (nit 6). |
| New 1: D022-310 against D022-243 | partly fixed | Fixed in ADR-0136:122-125, :230-238; research.md:3677-3700; plan.md:1222; tasks.md:1124. But the Signal-with-Start clause contradicts D022-310, T022-239's handler cannot publish an event with its credential, and tasks.md:1116 still says "a handler starting fresh runs deferred by the budget" (new finding 5). |
| New 2: the conversion and the wildcard arm | fixed | D022-315 and D022-316 (research.md:3429-3472); tasks.md:1282-1283. Premises hold at error.rs:360-369 and identity.rs:31-55 (no `#[non_exhaustive]`). |
| New 3: `RUN_RECOVERY_BINDINGS_EXIST` | fixed | Struck everywhere: contracts/errors.md:73, plan.md:1110, :1208, data-model.md:876, tasks.md:1242, :1249, :1251, research.md:3214-3217. Nothing left in the OpenAPI contract. |
| Nit 4: audit vocabulary written by SQL | fixed | research.md:3701-3707; tasks.md:1277 |
| Nit 5: `.permissions()` | fixed | research.md:3516-3530. The rename's callers match the tree: api-http routes/identity.rs:51, execution.rs:1394, declared_operations.rs:252, domain identity.rs:710. The other `.permissions()` calls (api_admission.rs:263, :274; pending_reset.rs:485, :628) are on other types. |
| Nit 6: "Enforced by" in ADR-0056 and ADR-0135 | fixed | ADR-0056:18-20; ADR-0135:6 |
| Nit 7: leftovers | fixed | ADR-0050:143, :220, :224; research.md:3403-3404 |
| Nit 8: decision 4's rejected options | fixed | ADR-0136:211-219 |
| K: acceptance not recorded | open, as intended | design-fixes-responses.md:185-186 |

### New findings

**1. Major: the root claim holds the worker's session row and can then run a lookup round. The `no-round` argument for `worker_session_room` is false.**
- **What the code does:**
  - `claim_root_run` locks the session row first: `worker_session_room`, `FOR UPDATE` (worker.rs:1146-1148, execution.rs:993-996).
  - In the same transaction it can then call `reject_input` (worker.rs:1253-1260, :1316-1324, :1360). That fails the step through `Engine::run` (worker.rs:1467-1486).
  - A failed step releases its `onError` successor (plan.rs:124, :218, :941). If it ends the run, it opens the handler region, whose nodes "are decided by the same lookup and gate rounds" (ADR-0136:62-64).
  - Either way a cacheable successor asks the lookup round, which takes the producer's binding key shared (ADR-0135:49-51).
- **The deadlock.** Often the claiming worker identity is also the producer of the cache entry. A binding replacement of that identity holds the key exclusively and waits on that identity's sessions `FOR UPDATE` (bindings.rs:88-92, :221-223), while the claim holds the session row and waits on the key.
- **Consequence.** PostgreSQL aborts either the claim or the administrator's binding replacement, which is exactly what ADR-0056 2(a) forbids. T022-271's domain test ("a lease never asks `CacheLookup`") stays green while this path deadlocks, so the fitness function would certify a broken rule.
- **Smallest fix.**
  - In `claim_root_run`, take the claimer's binding key shared (`hold_worker_binding`) before `worker_session_room`, the order the scoped claim already uses (scoped_claims.rs:102 then :112).
  - Add a fourth argument kind, `key-held`: the row's subject's binding key is held shared before the row lock. The round's lock on that key is then a re-entry, and a replacement of another subject never locks this row.
  - T022-271: replace the lease test with a `slot_order.rs` case. A root claim refuses an input whose `onError` successor is cacheable with an entry the claimer produced, run against a replacement of the claimer's binding, in both orders.
  - For a `no-round` entry reached through a `RunTx` method, the rule must name the method's application callers.

**2. minor: the argument for `PgRunTx::database_owner` reaches the right conclusion from the wrong premise, and its planned red demonstration cannot turn red.**
- **Premises that do not hold** (research.md:3668-3676):
  - bindings.rs:91-93 is the end of the namespace key lock, not the one-profile rule. That rule is at :93-100 and :154-158, and it applies to managed sessions only (`profile_matches = !managed || ...`, workers.rs:293-299).
  - `accepted_protocol_minor >= 6` does not identify a database executor: an ordinary scoped worker on a database-capable store also negotiates 6 (worker.rs:82-84).
- **What actually keeps it safe.**
  - `database_owner` takes `binding:{tenant}:{namespace}:{subject}` shared before the session row (database.rs:38-47), so the case is `key-held` (finding 1).
  - Only the database-runner role offers managed activities (worker.rs:105-108, workers.rs:300-309).
- **Consequence.** T022-271's "shown red once with … the executor's subject made the producer" will stay green, because the key is already held.
- **Fix.** Argue the entry as `key-held`, citing database.rs:38-47. Make the hand-made red step remove the lock at database.rs:40-47 instead.

**3. minor: `after-rounds` is false in transactions that run the engine twice.**
- **Where.** The argument says the calling functions "run after every round of the commit" (research.md:3595-3597), but:
  - `decide_grant` runs `Engine::run` and then `deliver_held_signal`, which runs `Engine::run` again (passes.rs:6261-6283, execution.rs:2275).
  - `finish_start` runs a reset, then `reapply` delivers signals through `deliver_signal` (runs.rs:2035-2059; reset.rs:741-769).
  - A Signal-with-Start runs `Engine::start` and then delivers its signal (runs.rs:2081-2084, :2158).
- **Why it matters.** The first `persist`'s step-row locks are held through the second call's rounds.
- **Why it is safe today.** Those rows belong to runs that no session has leased.
- **The risk.** The script's check ("called only from `persist`…") would accept a future second engine call after a `persist` that locked a `LEASED` row.
- **Fix.**
  - Reword `after-rounds` as "after every round of the transaction, or the rows written cannot be `LEASED` or `RUNNING`".
  - Name the three transactions above with that argument.
  - Make check (2) refuse a function that reaches two engine entry points unless it is listed.

**4. minor: the lock forms miss two statements that lock rows.**
- `INSERT INTO worker_sessions ... ON CONFLICT ... DO UPDATE` (workers.rs:329-344, registration) locks the row it conflicts with.
- `DELETE FROM worker_sessions` (bindings.rs:255; maintenance.rs:1218) locks rows as `FOR UPDATE` does.
- None is on a deciding path today, so the current list is complete in effect. A future one would pass the check unseen.
- **Fix.** Add both forms to D022-325 and list the three statements as `no-round` (registration, the replacement itself, the expiry purge), with negative fixtures.

**5. minor: the Signal-with-Start clause of D022-327 contradicts D022-310, and T022-239 cannot be built as written.**
- **The contradiction.** D022-310 makes "every admission of a principal that holds `RunRecovery` (start, reset, rerun)" a handler-tree run (research.md:3382-3384). A Signal-with-Start is only a start through the caller's principal (`StartVia::SignalWithStart`, event.rs:487; `RunService::signal_with_start`, runs.rs:1045). No workflow node can perform one (workflow.rs:669-780). So a Signal-with-Start the handler makes is a recovery start, yet ADR-0136:124-125, :237-238 and research.md:3683-3685 call it ordinary.
- **T022-239.** Its handler "publishes an event" (tasks.md:1124). `RunRecovery` holds no `event:publish` (ADR-0136:73-77; identity.rs:491), and no node publishes an event.
- **Fix.**
  - Strike "or a Signal-with-Start" in both places.
  - State what triggers the run: an activity using a separately held `event:publish` credential, or an outside system. T022-239's test setup names it.
  - Correct tasks.md:1116.

**6. nit: the allow list cannot be keyed by function alone.**
- Several functions have statements in two argument kinds:
  - `checkpoint_collection_iteration`: primitives.rs:2275 and :2362.
  - `drain_collection_iteration`: :3026, :3140, :3168.
  - `PgLoopProgressStore::checkpoint`: loop_progress.rs:1850, :1947, :1976.
  - `drain_iteration`: :2961, :3026.
- The claim's `FOR UPDATE SKIP LOCKED` is the module constant `RUN_FOR_CLAIM` (execution.rs:2590-2594, used at :401), not inside `claim_reservation` (:2462).
- **Fix.** Key each entry by function and statement (a marker comment next to the literal), and key constants and macro arguments by constant name.

**7. nit: check (2) as worded has gaps.**
- rustfmt writes `self` and `.run(` on separate lines (execution.rs:2324-2325, :2368-2369, :2378-2379), but the line-break allowance is stated for `engine` only.
- A receiver bound to another name escapes.
- `Engine::run_addressed` asks no round today (execution.rs:2005-2032), and ADR-0056:127 names `Engine::run` only.
- **Fix.** Match `.run(` and `.run_addressed(` by the transaction argument rather than the receiver's name. Say that `run_addressed` is included as a precaution.

**8. nit: the unbind marker is transaction-wide.** `sagat.unbind_audited = 'on'` (research.md:3534-3537) silences the trigger for every later binding delete in that transaction. Set it to the binding's key, and have the trigger compare against that key.

**9. nit: citations.** D022-317's backfill filter is at schedule_firing.rs:459, not :456.

Main files:
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0136-failure-handlers-run-between-the-stop-and-the-terminal-commit.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/docs/adr/ADR-0135-a-step-cache-hit-is-a-journaled-completion-decided-before-the-gates.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/worker.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/execution/database.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/bindings.rs
