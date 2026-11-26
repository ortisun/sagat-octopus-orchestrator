# Design re-read of decision D041-121 (the fourth run's setup, "at most twice", option 1c, the (2b) reading rule)

Reviewer: `egm:egm-design` subagent, read-only, on the snapshot of `21e4bd74`. Saved verbatim.

**Scope and limits.** I read files only from the snapshot at `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot`. I had no shell, so I could not check that its commit is `21e4bd74`, and I could not run the tests. I read:
- the diff since `9361a112`;
- my previous re-read;
- the design: section 2b, "What a report can wait for", T041-35 with its fourth run, T041-36, Risks, Complexity Tracking;
- ADR-0097 (decisions 3b, 4, 8, option 1c, the acceptance clause);
- ADR-0095 decisions 3 and 5;
- research.md (D041-120, D041-121, G041-021 to G041-027, O041-038 to O041-042), data-model.md §3b, §7 and §8a, validation-report.md (the D041-121 section);
- the prototype: `step_log_spike.rs` (prewarm, the hook's tagging, `run_row`, the commit-wait sampler), `main.rs` (`percentile`, `over_p99_by_path`, `API_POOL`, the row JSON), `step_log_leases_tests.rs` (the new setup test), `.github/workflows/perf-spike.yml`.

No target standard was supplied beyond your task. I judged against its questions and against constitution principle VIII ("stated as it is").

Short names (paths below the snapshot root):

| Short name | Path |
|---|---|
| design | `docs/designs/041-step-logs.md` |
| ADR97 | `docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md` |
| ADR95 | `docs/adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md` |
| research | `specs/041-step-logs/research.md` |
| dm | `specs/041-step-logs/data-model.md` |
| spike | `tests/performance/scalability-ha/step_log_spike.rs` |
| main | `tests/performance/scalability-ha/main.rs` |
| leases | `tests/performance/scalability-ha/step_log_leases_tests.rs` |

## Overall

**Closed:**
- **Blocker 1:** the setup now folds ended leases on a connection the fill holds.
- **Majors 1 to 4.**
- **Most of Major 5:** the share rule, which could never name the refill, is replaced. The new rule tags completions, has three branches and a declared expectation, and its arithmetic is right.

I checked every number in the changed text of the fourth run. All are correct (details in minor 1).

**One Major remains, and it has to be fixed before the first round.** The new (2b) rule only sees the refill's *direct* cost: the completions whose own carry refilled. It cannot see the *indirect* cost, which is the likelier cause in this spike: a fallback refill holds the tenant's usage row until its report commits, and every other (a) completion queues on that row at its commit. This is the cause I named in Major 5 of the last re-read, and it is still unseen. The harness cannot recover this afterwards: the run has to record it, so the rule and the harness must change before any round.

No new Blocker.

## Status of the previous findings (re-read of `9361a112`)

| Previous finding | Status |
|---|---|
| Blocker 1 (the setup's fold on a full pool) | **Closed.** spike:1805-1811 runs the fold on `held.first_mut()`. Setup leases answered `busy` or `contended` are retried at most 10 times (`0..=10`, one try and ten retries, spike:1814), and anything else asserts (spike:1826-1829). The share index runs across replicas: `prewarm_from(…, replica_index × API_POOL)` (spike:1940-1946), with `API_POOL` = 20 (main:73), as the design states (*i* = 20*r* + *j*, design:2307-2309). The new test `the_setup_lease_fills_a_full_pool_and_folds_ended_leases` (leases, diff :984-1041) is a plain `#[tokio::test]` on a pool of 2. It checks the fold and both shares, and validation-report.md records that it failed on `9361a112` with the exact pool-timeout error. Remaining: it is compiled only under `perf-spike` (spike:739), and the perf-spike workflow only builds (`--no-run`, perf-spike.yml:60), so the gate job never runs it (minor 4). |
| Major 1 ("at most once"; `busy` back under the run lock) | **Closed.** "At most twice, once under the run lock" is stated in ADR97:130-132 and :303-304, ADR95:104-105, design:913-915 and :1229-1235. I checked it also holds with the content finalizer at commit: a report that fell back already holds the row, so it is two waits at most either way. Passing `busy` to the carry is put to the owner with O041-041 (research:215, design:1233-1235). The longer ensure budget is rejected with a reason (research:132). T041-36 tests the second wait with `pg_stat_activity` (design T041-36, "after an ensure answered `busy`…"). |
| Major 2 (option 1c; the pooler regime) | **Closed.** The deadlock argument is withdrawn and what 1c gains and costs is weighed (ADR97:227-236). ADR97 3b (:142-145) and design 2b say decision 3b has no effect behind a transaction pooler. G041-027 is in O041-042 (research:216). There is a counter for affinity misses and a runbook entry (T041-25). The trigger to reopen it names the wrong metric (minor 5). |
| Major 3 (stale texts) | **Closed** for every cell of my table: ADR97:145-146, decision 8, ADR97:69, design:25, FR-041-005, L4, section 2a, the risk row, ADR95:150. A few texts outside that table are still stale (minor 6). |
| Major 4 (ADR-0095 "whatever the result") | **Closed.** design:2370-2372 puts only ADR-0097, O041-040 and O041-041 to the owner whatever the result, and ADR-0095 / O041-038 once T041-35 passes. This matches D041-115, O041-038 (research:213) and the "Then" step (design:2224-2228). |
| Major 5 (the (2b) rule was predetermined; the likeliest cause left out) | **Partly closed.** The rule is new, with three branches and a declared expectation; its arithmetic is in minor 1. Still open, and still Major: refills' indirect effect on other completions, and branch (ii) does not discriminate (Major 1 below). |
| minor 2 (shares repeat per replica; no share for (f), (g), (d)) | Closed: indexed across replicas, (h) stated "1 × 16 only", and "no share is declared for (f), (g) and (d)" (design:2318-2319), with G041-025 limited to (a) and (h). The multiplier is still close to 7/15. At 160 distinct indexes its clusters overlap, so the fill levels are close enough to uniform. No change needed. |
| minor 3 (G041-025 consequences) | Closed (design:2363-2366, research:162). |
| minor 4 (G041-021 denied the timeout path) | Closed (research:158; FR-041-005 residual, design:161-163). |
| minor 6 (Studio's "not your quota") | Closed (design:1272, :1797-1798; contracts.md; ADR97 Consequences). |
| minor 8 (where the pid comes from; `charged_at`) | Closed (design:1236-1241; dm §3b and §8a). |
| minor 11 (Complexity Tracking) | Closed (design:1923). |
| Nits 1, 2, 4 | Closed. Nit 3 is moot under the new rule. Nit 5 was not re-checked. |
| minors 5 (1g), 7 (`invalid` threshold), 9 (limit-change herd), 10 (T041-9 consequences) | Not addressed by this change, apart from O041-041's new rule for raising it again (research:215), which answers part of minor 10. They are still open as Minors and do not block the run. One coupling is new: if minor 7's tighter `invalid` threshold is ever adopted in the spike, the setup's 1 MiB ensure (spike:1816) would answer `invalid` and every row would stop (Nit 3). |

## Blocker

None.

## Major

### Major 1. The (2b) reading rule at 8 × 128 cannot see the refill's effect on other completions, and its branch (ii) fires for almost any failure

**Where:**
- the rule: design:2348-2357;
- the spike's refills run inside the report: design:2303-2306;
- the design's own statement that a report's commit waits behind "another report's ensure or fallback": design:922-923;
- the tagging covers only the carry's own step: spike:1590-1594;
- every carry sets the carry `application_name` for its transaction, which is what the sampler filters on: spike:1573 (`CARRY_APPLICATION`) and spike:2083-2091;
- the printed counter: main:2284 (`commit_waits_by_carry`).

**The mechanism the rule misses:**
1. In the spike, every refill after the setup is a fallback inside the report transaction (design:2304-2306). That is the "conservative" choice the design makes on purpose.
2. A fallback refill locks the tenant's `tenant_retained_usage` row and keeps it until its report commits: the rest of the carry (CC to CE), the rest of the report's statements, then the commit.
3. Every (a) completion stores a 1 KiB output, so every (a) completion takes the same row at its commit (G041-024). While a refilling report holds the row, every other (a) completion of the tenant queues behind it. design:922-923 names exactly this wait.
4. Rough size: about 28 refills a row, each holding the row for something near the carry's p99 at 8 × 128 (about 32 ms) plus the end of the report. That is on the order of 1 s of a stalled commit queue per row, spread over several percent of the completions.
5. The gate's declared margin is about 5 ms (+56 ms expected against an allowance near 61 ms, design:2340-2341). The indirect effect can therefore decide the gate on its own. The built design does not have it: there, the ensure commits right after its refill, under no run lock.

**What the rule does with that outcome:**
- **(i)** removes only the completions whose *own* carry refilled or was answered mark-only. The queued completions are untagged and stay in, so (i) does not fire.
- **(ii)** fires whenever the excess over the allowance is at most (a)'s fast-path carry p99, about 32 ms. A failure at a margin of about 5 ms always lands here. The rule then names "the lines' p99 (section 3's chunk rows, decision D041-111)", a redesign D041-111 rejected for the read paths, for a cost the spike creates and the built design does not have.
  - The test is not discriminating: "the excess is no larger than the carry's p99" shows only that the carry's own time *could* explain it, not that it does.
  - "Expected, if it fails: (ii)" is honest, but it confirms that the branch is decided before the data.
- **(iii)** reports `commit_waits_by_carry` as "the environment question". In (a) every report carries, so the sampler's blocker filter (`b.application_name = CARRY_APPLICATION`, spike:2087-2088) counts every commit wait on the content queue. It cannot tell a refill holder from an ordinary content finalizer (G041-024), and neither is the environment.

**Fix** (before the first round; the data cannot be recovered afterwards):
1. **Make refill holders visible.** On its refill path, have the carry function set a distinct `application_name` for the rest of its transaction, for example with `set_config('application_name', '<carry-refill>', true)` after the refill succeeds. Add a third filter to the sampler, `commit_waits_by_refill`, alongside `by_carry`. Or record each refilling report's interval from the refill to its commit (the hook already times commits when `timed`), and tag the completions whose report committed inside one.
2. **Add a branch before (ii):** if commit waits behind refilling reports, or the completions that committed inside a refill hold, account for at least half the excess, the cause is the spike's in-report refill. Declare the consequence: the built ensure removes it, so record it as not decided for (2b) at 8 × 128 rather than a design change, and say how T041-35 can then pass (for example, a fifth run with an ensure in the spike's hook, or the owner's answer).
3. **Replace (ii)'s test with one that discriminates:** the p99 of (completion time − its own carry time) against the allowance. If that meets the allowance, the carry's own statements are the lever (the lines among them). If it does not, the cause is outside the carry's own time and (iii) applies. This needs the hook to keep each carry's time by step, as it already keeps `slow_steps`.
4. **Rename (iii):** the content queue (G041-024) and the runner's load, not "the environment" without qualification.

If the coordinating session prefers not to change the harness: declare instead that a (2b) failure at 8 × 128 by less than the carry's fast-path p99 is *not attributable* by this run. Record it as not decided and triggering no design change, rather than as (ii).

## Minor

### minor 1. The numbers of the changed text: checked, and they hold

**Where:** design:2307-2321, :2328-2329, :2348-2350; research:132.

| Claim | Check |
|---|---|
| (a) per row: 9,000 × 3,280 = 29,520,000 bytes; / 1,048,576 = 28.15 refills; 0.31% | Correct. One grant serves 319 carries, so 9,000 / 319 = 28.2 matches. |
| (h): 9,000 × 64 × 328 / 1,048,576 = 180.2; 2.0% | Correct. With the lease folded whole, 49 carries a grant, it is 183.7 refills (2.04%). This is within the "twice the share" margin. |
| 29,520,000 / (160 × 1,048,576) = 0.176 | Correct (0.17595). |
| *i* = 20*r* + *j* | Matches `API_POOL` = 20 (main:73) and spike:1944. *j* is 0-based in the code (Nit 1). |
| "index ⌈0.99 *n*⌉ − 1" | Matches `percentile` (main:301), which `over_p99_by_path` now uses (main:2356). Nit 4 of the last re-read is closed. |
| "at most 90 carries above the p99 of a row of 9,000" | Correct: 9,000 − 8,910. |
| "9 mark-only answers a valid round allows" | Correct: "more than 0.1%" voids, so 9 is allowed. |
| 37 / 90 = 41% | Correct (41.1%), below the old one-half threshold, so the old rule could never name the refill. |
| Setup test: shares for *i* = 3 and 4 | Consistent with `setup_share(first + i)`, first = 3. |
| validation-report: 35 lease tests; 51 in the binary | Consistent with 34 + 1, and 46 + 5 across D041-120 and D041-121. |

### minor 2. The (2b) branches do not say which figures they compare
**Where:** design:2351-2354.

The gate uses the median of three valid rounds per size (design:2244-2245, D041-109), and four runners in a fixed order. Branch (i) compares "(a)'s `complete_p99_without_slow_paths_ms` minus off-1K's p99", and (ii) "the excess" against "(a)'s fast-path carry p99". The rule does not say whether these are per round, medians of the three rounds, or pooled. The judge script does not implement them, so they will be applied by hand.

**Fix:** state "the median over the three valid rounds used for the gate, each round's off-1K first figure", the same way for (i), (ii) and the threshold.

### minor 3. The invariant on `complete_slow_paths_tagged` has no consequence and no test
**Where:** design:2332-2333; spike:1590-1594 and the `run_row` tagging.

The count "must equal the row's `written:refill` and `mark_only` carries". Nothing says what happens when it does not: the hook tags by `a.step` in a `HashSet`, so a step carried twice would be tagged once. No test checks that the tagging matches (for example, a two-step row with one forced refill).

**Fix:** declare that a mismatch voids the (2b) attribution for that row (not the round). Add a non-ignored check, or assert it in the harness.

### minor 4. The gate job does not run the tests that guard its setup
**Where:** spike:739 (`cfg(all(test, feature = "perf-spike"))`), `.github/workflows/perf-spike.yml:60` (`--no-run`), validation-report.md (local runs).

The new setup test is the one that would have caught Blocker 1. It runs only where someone passes `--features perf-spike` by hand, and the commit the gate measures is not required to be the one tested.

**Fix:** have perf-spike.yml run the binary's non-ignored tests before `carry_spike_rounds.sh`, or have the fourth run's record name the commit on which the 35 lease tests passed.

### minor 5. Option 1c's trigger to reopen names one metric and the threshold another
**Where:** ADR97:235-236 ("fallbacks above the reopening threshold of question O041-041"); research:215 (the threshold is "busy answers exceed 0.1% of carries").

Behind a transaction pooler, fallbacks are expected on almost every short lease, while busy answers come only past *b*. These are different rates.

**Fix:** state 1c's trigger on `step_log_quota_fallbacks_total{caller="carry"}` with a threshold of its own, or tie it to busy answers explicitly.

### minor 6. A few texts still list the old causes of a fallback, or the old staleness rule
- design:1129 (the budget table, the ensure's row): "older than 9 minutes". It is now "stale 9 minutes after the lease's last charge or grant" (design:1238-1241, ADR97:117-118).
- design:1252-1254 (the fallback refill) and design:1885 (the subtransaction risk row: "Only carries whose room a reclaim took after the ensure open one"). Both omit the ensure answers `busy`, `contended` and `error`, and the pooler case, which design:1229-1235 and :1047-1049 now list.
- design:1126: "a carry ignored by C0 to C2" costs "as today". A carry C0 refuses may have paid an ensure (design:1214-1215).

### minor 7. Branch (i) has no declared way forward for T041-35
**Where:** design:2352, :2356-2357.

Branch (i) names "the built ensure, ADR-0097 decision 3b, or option 1e". The ensure is already in the design, so there is no design change to review. T041-35 would remain failed, and O041-038 (ADR-0095) waits for it to pass. The fix to Major 1, point 2, should also cover this branch.

## Nit

1. design:2308-2309: "replica *r*'s *j*-th connection has *i* = 20*r* + *j*". Say that *r* and *j* count from 0, as the code does (spike:1812, :1940).
2. design:2365: "Refills above twice the share mean that collisions are back". With leases keyed by pid, collisions cannot happen by construction. Say "the lease's ownership or grant does not behave as designed (a design defect)".
3. spike:1816: the setup ensures 1,048,576 bytes, which passes only because the `invalid` threshold is the attempt's ceiling (open minor 7 of the last re-read). Note the coupling next to that threshold.
4. ADR97:25: "with its own budget". Elsewhere it is "the same budget" (ADR97:123, :156). Use one wording.
5. ADR97:278 and design:1053: "rare" is unqualified. Add "far from the limit and with session affinity" as elsewhere.
6. design:2370-2371 cites "ADR-0097's acceptance clause" for O041-041, but that clause (ADR97:6-11) names only O041-040.
7. spike:1814-1825: the `contended` retries have no pause. A fence held for a moment would use up the 11 tries in microseconds; a short sleep would make the retry meaningful.
8. design:1365 and ADR97:132-133: after `error` from a lost connection, the store must acquire another connection, so another backend, for the report. Say so: it is the same case as an affinity miss.

## Files

Read:
- `docs/designs/041-step-logs.md`
- `docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md`
- `docs/adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md`
- `specs/041-step-logs/{research.md,data-model.md,validation-report.md,contracts.md}`
- `tests/performance/scalability-ha/{step_log_spike.rs,main.rs,step_log_leases_tests.rs}`
- `.github/workflows/perf-spike.yml`

All are below `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot/`. I also read the diff at `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/diff-9361a112-21e4bd74.diff`.

Verdict: **not ready to start as declared**: 0 Blocker, 1 Major, 7 Minor, 8 Nit. The fourth run can start once Major 1 is fixed: either the refill-holder tagging and the reworked branches, or the "not attributable" rule. A re-read limited to that change is enough; Blocker 1 and Majors 1 to 4 of the last re-read are closed.
