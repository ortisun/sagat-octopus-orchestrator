# Architecture re-review: task T022-078 (the rate gate's reads, after the fixes)

Reviewer: `architecture-reviewer` (read-only, diff and spec only), 2026-11-20, over the uncommitted fixes on top of
5b2e6de4 on `feature/022-throughput-control`. Text as returned; resolutions in `rate-reads-responses.md`. The diff handed
to the reviewers was taken while a hand mutation was applied to the working tree (see the responses, row one).

**Re-review of the fixes to task T022-078 (feature 022, the rate gate's reads)**

I only read files. I ran no test, script or `git` command, because I have no shell here. Line numbers below are in the worktree `/Users/marcelo/Documents/Projects/sagat-wt-022`; "diff" means `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/review078.diff`.

## Verdict

Not approvable as the diff stands. One Blocker, and it looks like a hand mutation that is still in place. The earlier Majors 3 and 5 are closed by checks that fail when broken. The earlier Majors 1 and 2 are closed in the SQL, but the test only half pins them. Once the Blocker is fixed and the Major below is done, it can be approved.

## Blocker

**The "waiters ahead" query of the rate place has lost one of the grant pass's three filters, and the diff and the worktree have lost different ones.**
- Where: `RATE_PLACE_SQL`, CTE `ahead`.
  - The diff (lines 418-419) drops `x.deadline_at > me.t` and keeps the status and pause filters.
  - The worktree, `crates/infra-postgres/src/flow_control.rs:1009`, now reads `AND x.deadline_at > me.t AND NOT r.paused`, so the status filter is gone.
- The grant pass uses all three filters: `gate_queue.rs:163-164, 180-181` and `maintenance.rs:217-218, 225-226`.
- Both texts contradict the SQL's own doc comment (`flow_control.rs:988-992`), the plan, and the `position` wording in both contracts.
- Consequence: `position` counts timed-out waiters (the diff's text) or the leftover waiter rows of ended runs (the worktree's text) as being ahead. `estimatedGrantAt` moves by the same amount.
- This looks like a sequence of hand mutations caught midway. If so, `rate_rest::the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate` should be red under each one: each filter alone moves D to position 2.
- Smallest correction:
  - Restore `AND x.deadline_at > me.t AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused`.
  - Rerun that test.
  - Record the three red results in `rate-reads-responses.md`.
  - Take the diff again before the next review.
- It is also a live example of why the copied eligibility text is risky (see the deferrals under Minor 4).

## Major

**The page-cost `EXPLAIN` test does not catch the purge-backlog regression it was written for.**
- Where: `tests/integration/flow-control-step-policies/rate_rest.rs:948-958`.
- In `EXPLAIN ANALYZE`, `Actual Rows` is the rows a node emits after its Filter, and it is averaged over loops.
- Hand mutation: put `flow_control.rs:960` back to `g.leased_at + make_interval(…) > now()`. Then:
  - the probe becomes an index scan on the six equality columns;
  - the Filter drops the 600 aged grants and emits 1 row;
  - `read <= 1` still holds and `nextSlotAt` is unchanged, so the test stays green.
- The same mutation on `leased_counting` (`:942`) also goes unseen.
- On the 51-key pages the grant subplans loop 51 times, so the average also hides one key that reads many rows.
- Consequence: three documents claim a check that does not fail when the code breaks:
  - the principles row, `docs/engineering/architecture-principles.md:47` ("whatever … the purge's backlog");
  - `plan.md:694`;
  - D022-068 item 3.
- Smallest correction, for the `rate_limit_grants` and `rate_limit_waiters` nodes:
  - assert that `Rows Removed by Filter` and `Rows Removed by Index Recheck` are absent or 0;
  - bound `Actual Rows × Actual Loops` by a stated total rather than the per-loop value;
  - run the mutation above and see the test go red.

## Minor

1. **The compile-time assertion only works one way.**
   - `flow_control.rs:929-930` ties `GRANTS_PER_KEY_PER_PASS` to the number 256. It does not tie it to the `LIMIT 256` written in the SQL at `:953`.
   - An edit of that SQL literal alone still builds. Raising it is caught only by the `EXPLAIN` test (see Minor 2); lowering it is caught by nothing.
   - The comment "a change of the bound fails the build here" claims more than the assertion does.
   - Fix: keep the number in one place. Use a `grants_per_key_per_pass!()` macro yielding `"256"`, splice it with `concat!` into the SQL, and compare a const-fn parse of it with the constant. A cheaper option is a const-fn byte search asserting the macro text contains `LIMIT 256)`.
   - Also, the `///` block at `:922-928` now documents the anonymous `const _`, not `rate_key_columns_sql!`. Move the assertion below the macro.

2. **The page-cost check does not run on the pull request that would break it.**
   - `scripts/select-test-targets.py:78-81` maps `crates/infra-postgres/` to the budget suite only.
   - So an edit of `RATE_KEY_LISTING_SQL` alone runs neither `EXPLAIN` twin nor `key_consistency` until release. This follows the accepted CI policy (OD-2026-11-06-2), so it is not blocking, but principles row 47 does not say so.
   - Smallest fix: add "runs at release" to the row. Mapping `flow_control.rs` to `integration_flow_control_step_policies` would catch it per pull request, but that costs CI time, so it is the owner's call.

3. **The timeout test does not pin the 2 s timeout.**
   - The test API's pool sets `statement_timeout` to 10 s (`tests/support/api.rs:305, 381`); production uses 5 s (`pool.rs:33`).
   - If you delete `SET LOCAL statement_timeout` at `flow_control.rs:825`, the place statement ends at 10 s. That is inside the 60 s bound at `rate_rest.rs:759`, so the test stays green.
   - What it does pin is "a failure leaves no place and the run is shown".
   - Fix, either of:
     - assert the response arrives within about 5 s of the reveal lock's release (a bound, not a sleep);
     - start this test with a pool `statement_timeout` above 60 s.

4. **The deferrals in D022-068 item 6 are acceptable, but they are not carried forward.**
   - On substance:
     - The eligibility-predicate deferral holds once the Blocker is fixed, because the "pass skips" test pins all three filters of `RATE_PLACE_SQL` by behaviour.
     - The gate-neutral permit state and the single handler are refactors of correct code.
   - On traceability:
     - T022-080 (`tasks.md:407`) and T022-111 (`tasks.md:475`) do not mention the deferred items.
     - `rate-reads-responses.md:10-14` still marks every item this diff closes as "Open: next change".
     - T022-080 is a fault suite, which is an odd home for a SQL refactor.
   - Still open and not recorded in item 6:
     - No test sends a cursor of a retired digest version to the rate listing. Deleting `crates/api-http/src/routes/rate_limits.rs:63` is caught by nothing.
     - The earlier nits on the redundant unleased scan (`flow_control.rs:935-946`) and on the trigger's lock strength.
   - Fix:
     - add an "Amended 2026-11-20 (D022-068 item 6)" bullet under each of T022-080 and T022-111;
     - update the responses table with each closing test and its mutation result;
     - add the retired-version test, or record it.

5. **The adapter now decides part of a business rule.**
   - `flow_control.rs:853-866` builds `PlaceReading` and calls `estimated_grant`, so the adapter decides "the pass will not grant this waiter".
   - `RunService::get` (`runs.rs:2376-2383`) then overrides the same field for the reveal rule and for a run that waits for both gates. So two layers decide one field.
   - `PlaceReading` carries real meaning, so rule 6 is not the issue.
   - Fix: have the port return `Option<PlaceReading>` and compute `RatePlace` in `RunService::rate_place`.

## Nit

- D022-068 item 2 (`research.md:1181`) says the log carries "the error's kind". `runs.rs:2403-2407` logs `%error`, whose Display includes the driver's `detail` for `Unavailable` and `Internal` (`error.rs:90-93`). That is harmless for this statement; either log a closed kind or change the text.
- The sequence begin, read-only, 2 s timeout now exists three times (`flow_control.rs:547, 689, 818`). The doc of `LISTING_TIMEOUT_SQL` (`:902`) still says it bounds "one page of the key listing". Extract a helper.
- `rate_rest.rs:941`, `assert_ne!(… "Node Type", "Sort")`, can never fail, because the top node is `Limit`. The `Index Name` check on `Plans[0]` does the real work.
- `rate_rest.rs:950`, `as_u64().unwrap_or(0)`: if the tests move to PostgreSQL 18, `Actual Rows` becomes fractional and these bounds pass without checking anything. Use `as_f64`.
- In both contracts, `estimatedGrantAt` does not list the case "granted between the read of the row and of its place", which `position` lists. The two documents' `keyLabel` descriptions differ (namespace grant against per location); that predates this diff.
- The new `data-model.md:617-619` bullet garbles its SQL with misplaced backticks. Some lines in `research.md:1181` and in the plan's listing paragraph are over-long.
- The rationale in D022-068 item 3, "a waiter beyond the first 256 waits behind them anyway", is not exact when paused or expired waiters are among the first 256. The contract text is accurate; soften the rationale.
- `ceilings.rs` `the_concurrency_live_keys_count_misses_only…` comes from the rate-gate re-review (OD-2026-11-20-11), not from T022-078. Commit it separately with its own `Refs`.

## What holds

- **Layers:**
  - The application side adds only `time`, domain types and `tracing`. The SQL stays in `infra-postgres`.
  - `PlaceReading` replaces a tuple and carries meaning.
  - The domain is untouched.
- **Earlier Major 3, closed and enforced:**
  - a `key_consistency` row scoped to the tenant (`tests/support/flow_control.rs`);
  - a `must(20_270_207_000_000, …)` line, so a missing column is a failure, not a pass;
  - a one-microsecond tamper test and a test that removes several waiters in one statement;
  - the principles row.
- **Earlier Major 5, closed:** the test of a run waiting for both gates and the test of `waitingOn: concurrency`.
- **Earlier Major 2, SQL correct:** the rewrite is equivalent (the half-open window is kept and the index can serve it), and the set of periods is bounded by 256 waiters in queue order. The waiter bound itself is pinned: 600 and 5,000 waiters would break `<= 256`.
- **Earlier minors and nits:**
  - Minor 1 is closed with a fixture test, and the "file gone" test now expects two errors.
  - Minors 5, 6 and 8 are closed.
  - Minor 7 is mostly closed: unit tests at each edge of `k` and of the deadline, the cursor tag tested both ways, the other group's cursor tested.
  - The earlier nits on `since`, `additionalProperties`, the word "eligible" and the terminal `GRANTED` are fixed.
- **Contract:**
  - The canonical document changes only descriptions. The feature 022 document now matches the canonical one.
  - The new null cases are on fields that were already nullable. The slice is unreleased, so no published reader is affected.
- **Documents first:** the plan, both contracts, the data model, research and the principles table move with the code.
- **Operability:**
  - The place is read in its own read-only, bounded transaction.
  - A failure is a warning, not an error, and adds no metric label.
  - Every join carries the tenant. There is no dependency on the broker.
- **Decisions:** D022-068 makes reversible read-model choices under OD-2026-11-20-6; no ADR is needed.
- **Scope:** no migration, and every file belongs to feature 022.

## Fitness summary

| Rule | Check | State |
|---|---|---|
| Domain and application free of SQLx and Axum | `check-architecture.py` layers | Holds |
| Position follows the pass's three filters | `rate_rest::the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate` | Check is good; code currently broken (Blocker) |
| No estimate when paused, past the deadline, or the instant is at or after the deadline | unit `no_estimate_for_a_waiter_the_pass_will_not_grant`; `an_estimate_after_the_queue_deadline_is_withheld` | Holds |
| k-th lease at each edge | unit `the_estimate_is_the_kth_lease…`; `each_waiter_is_estimated_from_its_own_counted_lease` | Holds |
| Page reads its own keys through its index | `rate_rest::a_page_of_rate_keys_reads_only_its_own_keys` | Holds for the index, sort, sequential scan and waiter bound; misses the grant filter regression (Major); runs only at release (Minor 2) |
| Listing's waiter bound equals the pass's bound | `const _` assertion | One way only (Minor 1) |
| `oldest_wait_at` equals its waiters' oldest | `key_consistency` + tamper test + `must` | Holds |
| A failed place read never fails `getRun` | `a_place_that_cannot_be_read_in_time…` | Holds; the 2 s timeout is not pinned (Minor 3) |
| Rate bodies match the canonical schemas | `conforms(...)` in `rate_rest.rs` | Holds |
| No key value in the rate route | `key_value_free_errors` + fixture | Holds |
| Cursor of one listing refused by the other | unit test both ways; integration concurrency to rate; other group | Holds; a retired-version cursor on the rate route is not tested (Minor 4) |
| Dual-gated run: place, no estimate; `waitingOn: concurrency` | two `rate_rest` tests | Holds |
| Single eligibility predicate text | none (deferred to T022-080) | Gap, recorded only in research (Minor 4) |
| Hot-path cost of triggers | plan paragraph; T022-089 | Gap by construction, now documented |
