# Design re-read, narrow: D041-121 point 9 after 8b12401f (head 18898a98)

Reviewer: `egm:egm-design`. Read-only, on the snapshot of `18898a98`. I had no shell, so I could not run the tests or the judge, and I could not confirm the commit.

**What I read** (paths below the snapshot root):
- The diff 8b12401f..head and my previous report (`rereview-3/egm-design-narrow.md`).
- `rereview-3/resolution.md`, the whole file.
- `docs/designs/041-step-logs.md`: lines 2218-2241 and 2290-2399.
- `specs/041-step-logs/research.md`: line 132 (D041-121), in the diff.
- `specs/041-step-logs/validation-report.md`: lines 415-530 (the 8 × 128 rates and commit waits), plus the new entry in the diff.
- `tests/performance/scalability-ha/step_log_spike.rs`: lines 1555-1611 (the carry hook, `carry_by_step`), 2040-2160 and 2240-2419 (the sampler loop, the attribution and `sample_commit_waits`).
- `tests/performance/scalability-ha/main.rs`: the lines that print the row JSON (2281-2289).
- `tests/performance/scalability-ha/carry_spike_judge.py`, the whole file.
- `tests/performance/scalability-ha/step_log_leases_tests.rs`: lines 71-118 (the helpers) and the new test (about 1896-1958).
- `docs/adr/ADR-0097-…md`: lines 237-242.

No target standard was supplied beyond the task. I judged against the task's questions.

## Verdict

**Both of my previous Majors are closed.** The changes add no Blocker and no Major.

Count: 0 Blocker, 0 Major, 4 Minor, 5 Nit. The fourth run can start as declared. Minors 1 to 3 are one sentence each in the rule, and are best fixed before the first round.

## Previous Major 1 (the (2b) branch order): closed

**Order.** The refill branch now comes first (design:2365-2371). Branch (i) fires on either of two conditions:
- the refill share of the commit waits is at least ½, or
- the completions without the slow paths meet the allowance.

Branch (ii) is reached only after (i) fails, and branch (iii) only after (ii) fails.

**Consequences.**
- (i) no longer claims that the built design passes. It names a fifth run with the ensure before each report as the test of it (design:2368-2371). This also closes my earlier Minor 1.
- (ii) is stated as an upper bound. Its lever is first measured by a `perf/041-diag-*` diagnosis, and becomes a design change only if the measured reduction covers the excess (design:2372-2375). That removes the direct path from a misattribution to a redesign.
- A row whose tagged count differs now has a declared consequence (design:2379-2380), which closes my earlier Minor 2.
- The unbacked expectation "(i) is not expected" is gone (design:2380-2381).

**Computable.** Every input is printed:
- `commit_waits` and `commit_waits_by_refill` in the row JSON (main:2284-2286);
- `complete_p99_without_slow_paths_ms` and `complete_minus_carry_p99_ms`, as before.

The judge computes the share for each round and its median over the valid rounds (judge:52-53, 81-84). Each test is a comparison of a median with 0 or with ½, read by hand.

**research.md:132.** Point 9 states the same order, the same tests and the same consequences.

## Previous Major 2 (the sampler counted only the head of the queue): closed

**Correctness** (spike:2394-2419).
- **Termination.** `depth` grows by one on each recursive step and is capped at `< 8`, so the recursion is bounded whatever the lock graph, cycles included.
- **Reach.** A depth of 8 is ample. The two queue shapes are:
  - A tuple-lock waiter is blocked by the tuple-lock holder, which is the head waiter, and soft-blocked by the waiters ahead of it. The head is blocked by the row's holder. That is depth 2.
  - After the first hand-over, most waiters wait directly on the holder's transaction id (validation-report:507-510). That is depth 1.
- **Through non-COMMIT backends.** Recursion follows `pg_blocking_pids(c.pid)` for any backend, not only COMMIT waiters, so a chain passing through a carry that waits on the usage row still reaches the holder.
- **Counted once.** `count(DISTINCT c.waiter)` counts each waiter once, however many paths lead to a refilling backend. The chain starts only from `waiters`, so `by_refill ≤ commit_waits`, and the share lies in [0, 1].
- **Measured database only.** `waiters` filters on `a.datname = current_database()`. The sampler runs on `w.db.pool`, the measured database (spike:2099). A blocker in another database cannot hold a row lock of this one, so the join on blockers needs no filter of its own.

**The test is a real test of the queue** (leases, about 1899-1958).
- The holder updates the row inside a transaction named `spike041carry:refill`, with a local `set_config`.
- Two `COMMIT`s are queued in turn behind it. Each runs a deferred constraint trigger that updates the row, so its `query` is exactly `COMMIT`. `waits_on_a_lock` enforces the order (leases:71-88).
- The first waiter takes the tuple lock and waits on the holder. The second waits on the tuple lock, so its only blocker is the first waiter. The head-only sample therefore gives `(2, 1)`, as validation-report states, and the chain gives `(2, 2)`.
- The sample runs from a pool connection, so it also checks what my earlier Minor 4 asked: the name set inside the transaction is visible in another backend's `pg_stat_activity`.
- It does not check the "counted once" property (Minor 4).

## Numbers

- **New figures, correct:**
  - `(2, 1)` against `(2, 2)`.
  - "Above half of the carries' refills": behind a pooler, an ensure answers `made` only when the backend it lands on is also short, so nearly every refill is a fallback and the ratio is near 1. With session affinity it is near 0. The trigger therefore separates the two regimes, which closes my earlier Minor 3. The trigger is consistent across ADR-0097:240, design:3063 and research:132, and `step_log_quota_ensures_total` exists (design:1227).
- **Unchanged and still correct:** 37 / 90 = 41%, 28.2 refills, 0.31%, 2.0%, and 0.176.
- **One count looks stale:** "51 tests" (Nit 1).

## Minor

1. **The ½ refill-share test is coarse** (design:2365-2366).
   - This threshold was my own fix 2. On re-reading I think it detects too little.
   - It compares the refill-caused waits with *all* of (a)'s commit waits. Most of those waits are the content class's queue, which off-1K has too: 893 to 15,115 samples at 8 × 128 (validation-report:459).
   - A refill queue large enough to move a p99 of about 500 ms by a few milliseconds can be well under half of that total. Rough estimate: 28 holds of tens of ms, at about 120 commits a second.
   - A miss falls to (ii). (ii)'s diagnosis would then measure the lines' lever on the spike with the in-report refills still present, and could "cover" an excess the ensure would have removed.
   - **Fix:** run (ii)'s diagnosis with the ensure in the spike's hook, so the lever is measured without the artifact. Alternatively, state the basis of the ½ threshold and accept that it detects only a dominant refill queue.
2. **(ii) has no declared consequence when the diagnosis falls short** (design:2374-2375). The rule says when the lever becomes a design change, but not what follows when its measured reduction does not cover the excess. Declare it, for example (iii)'s consequence.
3. **A void attribution and the refill share can disagree** (design:2379-2380; judge:43-53).
   - The share does not depend on the tags. The judge computes it, and prints its median, even for a row whose attribution is void.
   - The design says such a row takes (iii)'s consequence, so a reader could apply either (i) (a fifth run is offered) or (iii) (owner, with the environment question).
   - **Fix:** state which one takes precedence.
4. **The test does not exercise "a waiter counted once"** (leases, about 1943-1947).
   - With two waiters, `count(*)` over the chain rows gives the same `(2, 2)` as `count(DISTINCT …)`.
   - With a third queued `COMMIT`, which is blocked by both the head and the second waiter, the reaches are (W3, H, 2) and (W3, H, 3). `count(*)` then gives 4, against the expected 3.
   - **Fix:** add the third waiter and assert `(3, 3)`.

## Nit

1. **The test count.** validation-report (end of the new entry) still says "51 tests, 10 ignored benchmarks". One test was added, and the binary's files now have 62 test attributes, 10 of them ignored, so 52 tests should run. This assumes no module is excluded by a feature flag.
2. **The queue wording.** spike:2390-2393 and design:2343-2345 say "every waiter but the first is blocked by the one ahead of it". More precisely:
   - tuple-lock waiters are blocked by the head and soft-blocked by every waiter ahead;
   - after a hand-over, most waiters are blocked by the holder's transaction directly.

   The chain handles both cases, so this is wording only.
3. **The depth cap.** `UNION` over `(waiter, pid)` without `depth` would compute the exact closure, still terminate because the set of pairs is finite, and need no cap of 8.
4. **The sampler's cost.** `pg_blocking_pids` is volatile and takes every lock-manager partition lock.
   - The existing `EXISTS` subqueries already call it once per `pg_stat_activity` row, for each waiter. That cost predates this change, applies to both sides of the gate, and is the larger one.
   - The chain adds about 4N² calls, for N tuple-lock waiters.
   - Computing the edges once in a `MATERIALIZED` CTE would bring this down to one call per waiting backend.
5. **The judge's output.**
   - Dropping `ms` from the attribution keys (judge:84) leaves the (i) and (ii) medians in milliseconds with no unit.
   - A round with `commit_waits == 0` silently drops the refill-share median, with no reading declared for that case. This is unlikely at 8 × 128.

## Files

All are below `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot/`:
- `docs/designs/041-step-logs.md`
- `specs/041-step-logs/research.md`
- `specs/041-step-logs/validation-report.md`
- `docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md`
- `tests/performance/scalability-ha/step_log_spike.rs`
- `tests/performance/scalability-ha/main.rs`
- `tests/performance/scalability-ha/carry_spike_judge.py`
- `tests/performance/scalability-ha/step_log_leases_tests.rs`
- `docs/designs/041-step-logs.reviews/2026-11-20-writer-leases/rereview-3/resolution.md`
- `docs/designs/041-step-logs.reviews/2026-11-20-writer-leases/rereview-3/egm-design-narrow.md`

The diff: `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/diff-8b12401f-head.diff`

**Ready to start as declared:** 0 Blocker, 0 Major, 4 Minor, 5 Nit.
