# Design re-read of D041-121 point 9 (the gate (2b) reading rule at 8 × 128 and its harness changes)

Reviewer: `egm:egm-design`, read-only, on the snapshot of `8b12401f`. I had no shell. I could not confirm the commit or run the tests or the judge.

**What I read** (paths below the snapshot root):
- The 21e4bd74..8b12401f diff and my previous report (`rereview-3/egm-design.md`).
- `docs/designs/041-step-logs.md` (abbreviated design): lines 2230-2392, T041-35's second to fourth runs.
- `specs/041-step-logs/research.md` (abbreviated research): line 132, D041-121 with its point 9.
- `specs/041-step-logs/validation-report.md`: lines 823-844.
- `tests/performance/scalability-ha/step_log_spike.rs` (abbreviated spike): lines 562-742 (the carry and its slow path), 1258-1660 (the hook, `carry_by_step`) and 2080-2405 (`run_row`, the sampler).
- `tests/performance/scalability-ha/main.rs` (abbreviated main): lines 2170-2360 (the row JSON and the row order).
- `carry_spike_judge.py` (abbreviated judge), whole file.
- `step_log_leases_tests.rs` (abbreviated leases): lines 997-1026.

No target standard was supplied beyond the task. I judged against its questions and against constitution principle VIII ("stated as it is").

## Verdict

**My previous Major 1 is only partly closed.**

What is now right:
- The harness names refilling reports, and the naming works.
- Completion minus own carry is computed correctly.
- The rule is judged on medians.
- Branch (iii) is renamed correctly.
- The rule is declared before any round: validation-report.md records no round of the fourth run.
- I found no wrong number.

What is still wrong:
1. **The rule (new Major 1).** The evidence about refill holders is read only in branch (iii), which is reached after (ii) has failed. The new test in (ii) still fires for almost any narrow failure, whatever its cause.
2. **The sampler (new Major 2).** It attributes each commit wait to the waiter's *immediate* blocker. In a PostgreSQL row-lock queue, that is the head waiter for every waiter except the head. So `commit_waits_by_refill` cannot count the queue behind a refill holder, and `commit_waits_by_carry` absorbs it.

Count: 0 Blocker, 2 Major, 4 Minor, 3 Nit. The fourth run should not start until both Majors are fixed.

## Answers to the questions

| Question | Answer |
|---|---|
| Does `set_config('application_name', …, true)` inside the carry's block last for the rest of the report transaction? | **Yes.** On a normal exit the block's subtransaction commits, and a `SET LOCAL`-style value passes up to the parent transaction. The function's `SET` clause names only `search_path`. PostgreSQL restricts a `SET LOCAL` to the function only for a variable the function's own `SET` clause names, so `application_name` survives the function's exit. The value is reverted at transaction end. The carry's top-level `set_config` at spike:643 already relies on this, and leases:1018 checks it inside the transaction ("x:refill"). |
| Does the sampler see it in `pg_stat_activity`? | **Yes.** The `application_name` assign hook reports the new name to shared memory at once (`pgstat_report_appname`). The revert at commit comes after the transaction's locks are released. Each sampler statement is its own transaction, so it reads fresh status. The test reads `current_setting`, not `pg_stat_activity` (Nit 1). |
| Is the name set only when a refill holds the usage row? | **Yes.** It is set only when the block's write returns `written` (spike:689-693). The mark-only paths roll the block back, which releases the row lock, and they do not set the name. A carry with nothing to charge takes the fast path (spike:679). |
| Is completion minus own carry computed correctly? | **Yes.** The fourth run uses `CARRY_RUN=3`, the one-call path (spike:1340-1342), which is the only path that fills `carry_by_step` (spike:1605-1607). The carry's time spans its whole client-side call, which lies inside the timed `complete()` call (spike:2260-2276). Completions and their steps are pushed together, so the zip lines them up (spike:2324-2329), and `saturating_sub` guards against a negative value. A carry that raised falls back to the whole completion time. One edge case is in Nit 2. |
| Is the rule computable from what is printed? | **Yes for (i) and (ii).** The judge prints per-round values and their medians against each round's first off-1K (judge:43-47, :79-82), which matches design:2362-2363. The branch is chosen by hand from the signs of the medians. **Not for (iii)'s figure**, which is wrong by construction (Major 2). There is also no declared reading when an attribution median is missing (Minor 2). |
| Is the rule declared before any round, with no wrong number? | **Yes.** "37 / 90 = 41%", "at most 90", "9 mark-only", 28.2 refills and 0.31% are unchanged and correct. "(i) is not expected" is an expectation with no backing (Minor 1). |

## Major

### Major 1 (previous Major 1, not closed). Branch (ii) still fires for almost any narrow failure, and the refill-holder evidence is read only after it

**Where:**
- design:2366-2375 (the order of the branches);
- research:132, point 9;
- judge:47.

**The test in (ii) does not discriminate.** Branch (ii) asks whether p99(completion − own carry) − off-1K's p99 meets the allowance A.
- Every carrying completion in the tail contains its own carry, of the order of the fast path's carry time. The fast path's p99 is 31.7-32.7 ms (design:2350).
- So p99(c − carry) ≈ p99(c) − C_tail, where C_tail is the carry time of the tail completions. Branch (ii) therefore fires whenever the excess over A is smaller than C_tail.
- The declared expectation is a margin of about 5 ms (+56 ms against about 61 ms, design:2351-2352). Any plausible failure would exceed A by a few milliseconds, and in that case (ii) fires whatever the cause.
- That includes the cause my last report named: other (a) completions queued at their commit behind a report that refilled. Those completions keep their queue time after their own carry is subtracted. Branch (ii) fires anyway, because subtracting their carry already covers the few milliseconds.
- This is the same behaviour as the old test ("excess ≤ fast-path carry p99"). My last report's fix point 3 proposed this test. It discriminates only when placed after a refill-holder branch (that report's point 2), and that branch was not adopted. The refill figure now appears only inside (iii).
- **Consequence:** (ii) prescribes "a design change, reviewed again" (the lines' p99, chunk rows, which D041-111 rejected for the read paths) for a cost that the built design's ensure removes. The design itself says so at design:2370.
- (ii) also measures removing the *whole* carry, which is an upper bound. A real lever recovers only part of it: chunk rows gained about 8 ms of 78 at p99, per D041-107 (design:2289).

**Fix** (before any round). Put a refill test before (ii), using one of these:
1. **Client-side, needs no sampler.**
   - For each `written:refill` step, record when its carry returned and when its `complete()` returned. That interval is when the report holds the usage row.
   - Tag every (a) completion whose `complete()` call overlaps a hold interval.
   - Compute (i′): the p99 over completions that are neither slow-path nor tagged. Removing a subset at random does not bias a quantile, so (i′) is a fair counterfactual for "no in-report refill holder".
   - If (i′) meets A, record the result under (i)'s consequence: not decided, then a fifth run or the owner.
2. **From the sampler**, once Major 2 is fixed: if the waits sampled behind refill holders, attributed by the root of each chain, are at least half of (a)'s commit-wait samples, use (i)'s consequence.
3. **Remove the artifact itself:** put the ensure in the spike's hook before any round, so that in-report refills happen only as true fallbacks.

In every case:
- State in (ii) that it shows only that removing the carry *could* close the gap. The design change it names must show a measured reduction at least as large as the excess.
- Update research:132, point 9, to match.

### Major 2 (new). `commit_waits_by_refill` counts only the head of the queue behind a refill holder

**Where:**
- the sampler: spike:2103-2111 (`b.pid = ANY (pg_blocking_pids(a.pid)) AND b.application_name = $3`);
- the design's claim that this counts "the commit waits sampled behind reports whose carry refilled": design:2342-2344 and :2370.

**Mechanism:**
- When a report holds the usage row and others queue for it, the first waiter takes the tuple's heavyweight lock and then waits on the holder's transaction id (here the block's subtransaction id, held by the refilling backend).
- Every later waiter waits on that tuple lock, so `pg_blocking_pids` returns the first waiter, not the holder.
- In (a), the first waiter is itself a carrying report named `spike041carry`.

**Result:**
- `by_refill` is at most one per refill holder per 10 ms sample.
- The rest of the queue the refill caused is counted in `by_carry`, which (iii) presents as "the content class's queue, gap G041-024".
- So the figure the owner receives under (iii) points away from the refill exactly when the refill is the cause. Any threshold test built on it (Major 1, fix 2) would almost never fire.

**Fix:**
- Attribute each COMMIT waiter to the root of its blocking chain: a recursive CTE over `pg_blocking_pids` until it reaches a backend that is not waiting on a `Lock`. Then filter on the root's `application_name`.
- Add a test with three backends: a refill holder and two COMMIT waiters, both of which must be counted behind the refill.
- Or rely on fix 1 of Major 1, which does not use the sampler. Even then, correct the description of `commit_waits_by_refill` at design:2342-2344.

## Minor

1. **Branch (i)'s rationale overstates the ensure** (design:2363-2366, :2375).
   - The built ensure *moves* the refill out of the report transaction. It does not take the refill's time out of the completion: the ensure's wait, refill and commit run inside the same report call, on the same completions.
   - So (i) firing is weak evidence that the built design passes. The fifth run is the right test. Say so, so that the "owner's answer" option is not taken on the premise that the cost disappears.
   - "(i) is not expected at the expected 28 refills" has no backing at a 5 ms margin. Refilling carries wait behind the content queue. If they sit in the completion tail, removing 28 of the 90 tail completions moves the effective percentile to about p98.6.
2. **No declared reading when the attribution is missing.**
   - The judge omits a median when any valid round's (a) row fails the tagged-count check (judge:45-49, :80-82). The rule says only "that row has no attribution" (design:2339-2340).
   - Declare it: (2b) at 8 × 128 is then not decided, with (iii)'s consequence.
3. **Option 1c's new trigger cannot fire for (a)**, at ADR-0097 option 1c and design:3057-3058.
   - Behind a transaction pooler, decision 3b does nothing, and fallbacks then equal the refill rate: about 0.31% of (a)'s carries (design:2324-2326). That is below the 1% trigger even when the ensure has no effect at all.
   - State the trigger relative to refills, for example fallbacks above half of the refills, or name (h)'s rate.
4. **The test checks less than the sampler relies on** (leases:1017-1024).
   - It reads `current_setting`, not `pg_stat_activity` from another backend.
   - Its fast-path assertion would pass even if `:refill` leaked past the commit, because spike:643 resets the name at the start of every carry.
   - Fix: read `pg_stat_activity` for the writer's pid from the pool, once while the transaction is open and once after the commit with no carry in between.

## Nit

1. The markdown table header and row in main still show "Commit waits / by ingest / by carry" (main:2205, :2292-2305). Only the JSON has `by_refill`.
2. `carry_by_step.insert` overwrites the first carry's time when a report is re-run (a downgrade or a retried transaction). Its completion then contains both carries, but only the last is subtracted (spike:1605-1607). Add the times instead, with `entry().and_modify(+=)`.
3. Previous findings that are closed:
   - Minor 2 (which figures are compared): closed by "the median over the three valid rounds…".
   - Minor 3 (a mismatch has no consequence): closed for the row; the test is still missing.
   - Minor 5 (option 1c's metric): restated, but see Minor 3 above.
   - Minor 6 (stale texts at design:1126, :1129, :1252-1254, :1885): closed.
   - Minor 7 ((i) had no way forward): closed by the fifth-run or owner option.
   - Nits 1, 2, 4, 6 and 8: closed.
   - Not re-checked: Nit 5 ("rare" without its qualifier) and Minor 4 (the gate job does not run the setup tests).

## Files

All are below `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot/`:
- `docs/designs/041-step-logs.md`
- `specs/041-step-logs/research.md`
- `specs/041-step-logs/validation-report.md`
- `tests/performance/scalability-ha/step_log_spike.rs`
- `tests/performance/scalability-ha/main.rs`
- `tests/performance/scalability-ha/carry_spike_judge.py`
- `tests/performance/scalability-ha/step_log_leases_tests.rs`
- `docs/designs/041-step-logs.reviews/2026-11-20-writer-leases/rereview-3/egm-design.md`

The diff: `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/diff-21e4bd74-8b12401f.diff`

**Not ready to start as declared:** 0 Blocker, 2 Major, 4 Minor, 3 Nit. A re-read limited to the (2b) branch order and the sampler's attribution is enough.
