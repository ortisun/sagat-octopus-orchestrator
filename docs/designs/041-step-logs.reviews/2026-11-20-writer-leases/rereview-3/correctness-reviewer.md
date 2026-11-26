# Correctness re-read of decision D041-121 (the setup on a full pool, the restated (2b) reading rule, the fourth run's declaration)

Reviewer: `correctness-reviewer` subagent, read-only, on `21e4bd74`. I read only the snapshot and the diff `9361a112..21e4bd74`. Paths are relative to the snapshot root `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot`. I ran nothing; the test results cited below are claims from `validation-report.md`.

**Verdict.** The fourth run can start. My Blocker 1 and Major 1 are both closed, and the change adds no new Blocker or Major. Two minors are cheap to fix and are best fixed before round 1, because the reading rule is declared before any round:
- minor 1: the (2b) rule does not say how its branches combine across rounds;
- minor 3: the judge does not void a round that stops in (h) or (d).

## Status of my previous findings

**Blocker 1 (the setup asks an emptied pool for a connection): closed.**
- The fold now runs on a connection the fill already holds, `held.first_mut()` (`tests/performance/scalability-ha/step_log_spike.rs:1805-1811`). The tenant is passed in as an argument (`prewarm_leased(pool, first, lease)`, `:1789`), so the test does not read the global.
- `the_setup_lease_fills_a_full_pool_and_folds_ended_leases` (`step_log_leases_tests.rs:1829-1889`) runs on a pool of 2 connections after a backend with a lease has ended. It checks three things:
  - the ended backend's lease is folded;
  - the tenant has exactly two leases, (1 MiB, u₃) and (1 MiB, u₄);
  - the invariants hold.
- Mutations it catches:
  - Going back to `.execute(pool)` times out after 2 s; per validation-report.md:834-838 this test failed that way on `9361a112`.
  - Dropping the fold leaves `pid_gone`'s lease in place.
  - Ignoring `first` gives u₀ and u₁.
- Its limit: it runs only with `--features perf-spike` (`step_log_spike.rs:739`), and `carry_spike_rounds.sh` does not run it before the rounds. That is acceptable, since the report records it run (51 tests).
- The grant is exactly 1 MiB: g = max(1,048,576, min(1 MiB, F/(2(A+1)))) with L = 2^40 (`step_log_spike.rs:361`).

**Major 1 (the (2b) rule could never name the refill): closed.** Rule (i) is a counterfactual on the completions themselves (design:2351-2352), and it can now name the refill.
- **The fields are printed:**
  - `complete_p99_without_slow_paths_ms` and `complete_slow_paths_tagged` (`main.rs:2268-2270`);
  - `carry_fast_p99_ms` (`main.rs:2276-2277`);
  - off-1K's `complete_p99_ms`, and the allowance max(10%, 2 ms), which `carry_spike_judge.py:34` already computes.
- **The tagging is correct:**
  - The hook keys on `ReportedAttempt.step = instance_step_id(version_id, run_id, ExecutionInstanceId::root(node))` (`crates/infra-postgres/src/execution.rs:7331-7343`).
  - The claim builds `ClaimedTask.step_id` with the same function, `instance_step_id(version, run_id, instance)` (`crates/application/src/worker.rs:1699`). For a root task the two are the same value.
  - The hook fires only for root reports (`execution.rs:7325-7328`). Every carry row is `Shape::Chain` (`main.rs:2228`), so each step id has one completion.
  - `calls == transitions` is asserted (`main.rs:2254-2258`), so every carry has exactly one completion.
  - A raised carry makes the report fail, which panics (`step_log_spike.rs:2248-2250`). So the "must equal" identity holds, except for `contended:<cause>` (nit 2).
  - The hook is built once per row (`step_log_spike.rs:1932-1934`), so `slow_steps` cannot leak from one row into the next.
- **The arithmetic is right:**
  - At n = 9,000, `percentile` takes index ⌈8,910⌉ − 1 = 8,909 (`main.rs:301`), so at most 90 carries are strictly above it.
  - (28 + 9) / 90 = 41.1%.
  - `over_p99_by_path` now uses the same `percentile` (`main.rs:2356`), which closes my earlier nit 1.

**Earlier minors and nits:**
- **minor 1 ("at most once"): closed.** The text is restated as "at most twice". minor 2 below is about what that bound leaves out.
- **minor 2 (`limit` with enforcement off): closed** in data-model:269-271.
- **minor 3 (a carry whose only row is a marker): closed** in data-model:599-602, with a case planned in T041-36 (design:2779-2780). The prototype still uses `v_kept = 0` (`step_log_spike.rs:679`); that is acceptable because no workload meets a ceiling.
- **minor 4 (a refusal's cause lost through the ensure): closed** (data-model:726-727).
- **minor 5 (the kept room aged from when it was seen): closed in the design.** The room now ages from the lease's `charged_at`, which each call returns. The value the reserve returns is the charge before this one, which is conservative.
- **minor 6 (`held` untested): open in the prototype.** A case is planned for T041-36 (design:2775-2776), but no prototype test bites the `program_limit_exceeded` handler.
- **minor 7 (the ensure's subtransaction): closed** (documented in ADR-0097:282-283 and data-model:685-686).
- **minor 8 (the `invalid` cap): open.** data-model:274-275 and `step_log_spike.rs:388` are unchanged.
- **minor 9 (the ensure's delay against the lease): closed** (FR-041-005 at design:168-170, G041-021).
- **minor 10 (T041-9 criteria): open.** Design:3034 still says "under 1,000 a second".
- **minor 11 (stale texts): mostly closed.** What remains is in nit 3.
- **Nit 1 (a prepared transaction and the own-lease `INSERT`): open.**
- **Nit 3 (`caller="ingest"` cannot be told apart): open.**
- **Nit 4 (the order of `a_completion_holding_the_usage_row_…`): open.** It is still listed at design:1289.
- **Nit 4 of the previous re-read (`lease_rows`): half closed.** It is now filtered by tenant (`step_log_spike.rs:2353-2359`). But the 64 start-pool leases belong to the same tenant, so they are still counted (nit 4 below).

## The fourth run's declaration against the prototype (checked)

- **Share index across replicas.** `run_row` passes `replica_index × API_POOL` (`step_log_spike.rs:1940-1946`), with `API_POOL` = 20 (`main.rs:73`). That matches design:2308-2309, i = 20r + j.
  - u_i = (i+1) × 489,905 mod 2^20. The multiplier is odd, so u_i is never 0 for i ≤ 159.
- **Retries.** `for _ in 0..=10` makes 1 try plus 10 retries, which matches "retried, at most 10 times". Any other answer, or the 11th `busy`/`contended`, panics. Per the validity rule that panic voids the round, but see minor 3.
- **Numbers in the changed text**, all correct:
  - 9,000 × 3,280 = 29,520,000 bytes, ÷ 1,048,576 = 28.15 refills (0.313%);
  - (h): 9,000 × 64 × 328 = 188,928,000 bytes, ÷ 1,048,576 = 180.17 refills (2.002%), at 1 × 16 only (`main.rs:2337-2340`);
  - 29,520,000 / (160 × 1,048,576) = 0.176;
  - ⌈0.99n⌉ − 1 matches `percentile`;
  - 37/90 = 41%.
- **The validity rule** is `carry_spike_judge.py:28-31`. Refills count as written (`count`, `step_log_spike.rs:1605-1607`), and up to 9 of 9,000 are tolerated (9/9,000 is not more than 0.001).
- **The G041-025 rule** can be computed from `carry_written_refill`, `carry_mark_only_quota` and `carry_mark_only_held`.
- **The alert threshold** in ADR-0097 decision 8 (0.1% for ten minutes) matches design section 9 (:1811-1812).

## Blocker

None.

## Major

None.

## minor

**minor 1. The (2b) rule does not say how its branches combine across the three rounds, or which "excess" branch (ii) means.**
- **Where:** design:2351-2356.
- **How the gate itself is judged:** on the median of three valid rounds plus two quiet rounds (design:2244-2245; `carry_spike_judge.py:54-59`).
- **What the rule leaves open:**
  - Is (i) the median over the same three rounds of (`complete_p99_without_slow_paths_ms` − off-1K p99 − allowance), or a per-round test?
  - Is "the excess over the allowance" in (ii) the excess of the full (a) p99 or of the counterfactual p99?
- **Scenario:** (2b) fails at 8 × 128 with medians close to the line (the declaration expects +56 against about 61 ms). In one round the counterfactual meets the allowance; in two it does not. A per-round reading names the refill, a median reading names the lines, and nothing declared picks between them. This is exactly the after-the-data choice that declaring the rule before any round is meant to prevent.
- **Why nothing catches it:** the judge does not compute the rule (`carry_spike_judge.py` has no field for it), so it is applied by hand.
- **Fix (one sentence, before round 1):** "each of (i) to (iii) is judged on the medians over the three valid rounds used for the gate; the excess in (ii) and (iii) is that of the counterfactual". Optionally, add those lines to the judge.
- **Confidence:** confirmed in documents.

**minor 2. "A report waits on the usage row at most twice, once under its run lock" leaves out the content finalizer at commit.**
- **Where:** ADR-0095:105, ADR-0097:132 and :304, design:1232, Risks row design:1867. Design:913 says "before its commit", and those other places drop that qualifier.
- **Scenario:** a report of (a) with 1 KiB output, while a release holds T's usage row.
  1. The ensure waits 200 ms and answers `busy` (wait 1, no run lock).
  2. The fallback refill waits 200 ms and answers `tenant_quota_busy` (wait 2, under the run lock). It never got the row.
  3. The commit's content finalizer then waits for the same row again, under the run lock and with no budget of *b* (wait 3; G041-024, "as today").
- **Why nothing catches it:** T041-36's planned case ("the report's fallback waits for the row once … so the report waited at most twice", design:2774-2777) uses a report and does not say it stores no output.
- **Fix:** qualify the bound as "for its step-log charge, before its commit". Add "a report that stores output also takes the row at commit (G041-024)", and make T041-36's case use an empty output, or assert all three waits.
- **Confidence:** confirmed in documents.

**minor 3. The judge counts as valid a round that stopped during (h) or (d), against the declared "a round that stops is void".**
- **Where:** `carry_spike_judge.py:40-43`:
  ```python
  x = m.get((l, 1))
  if x and x["carry_calls"] and ...
  ```
  A missing (h) or (d) row is skipped, not voided. The declaration is at design:2313-2314 and :2324-2325.
- **Scenario:**
  1. The round's 1 × 16 and 8 × 128 rows complete.
  2. (h), the workload with the most refills (2%), then stops: its setup asserts, or a carry raises and `failure("complete")` panics.
  3. The judge counts the round valid, and `carry_spike_rounds.sh:27-29` stops after three such rounds.
  4. The gates are then decided on that round, and G041-025's "in every round used, the refills of (a) and (h)" cannot be applied to it.
- **Why nothing catches it:** the exit status of `run.sh` is only logged (`carry_spike_rounds.sh:26`).
- **Fix:** in the judge, set `valid = False` when the (h) or (d) row is missing, or when the round's exit status is non-zero.
- **Confidence:** confirmed in code. The judge predates D041-121, but D041-121 makes setup failures a declared cause of a void round.

**minor 4. Rule (i)'s counterfactual misses delays a refill imposes on completions it does not tag, so it under-reads the refill.**
- **Where:**
  - `step_log_spike.rs:331-332`: the fallback refill locks `tenant_retained_usage` FOR UPDATE in the report transaction, and holds it to commit.
  - design:2351-2352, rule (i).
- **Scenario:** at 8 × 128, every (a) completion stores 1 KiB, so its commit takes the same row for the content class (G041-024).
  1. While a refill's report finishes CD, CE and its commit, the other completions' commits queue on that row.
  2. Those completions are not tagged, so they stay in `complete_p99_without_slow_paths_ms`.
  3. So (i) can fail to name the refill when the refill is the cause, and the rule falls to (ii), the lines, which is a redesign D041-111 already weighed.
- **Size:** about 28 refills, each extending a hold by a few milliseconds, against an off-1K p99 near 610 ms (allowance ≈ 61 ms). The effect is likely small, but its sign is fixed.
- **Why the printed data cannot separate it:** `commit_waits_by_carry` counts waits behind every carrying report, not behind refills (`step_log_spike.rs:2086-2091`, `:643`).
- **Fix:** in the slow-path block, set a distinct `application_name` before the refill and sample waits behind it. Or state in the rule that (i) is a lower bound on the refill's effect.
- **Confidence:** plausible.

## nit

1. **The setup retry has no pause.** `contended` returns at once, so 11 tries take microseconds and cannot outlast a holder (`step_log_spike.rs:1814-1825`). Nothing in the benchmark holds the fence, so this is harmless.
2. **The tagged-count identity is stated but not asserted, and one cause can break it.**
   - `complete_slow_paths_tagged == written_refill + mark_only` is not asserted the way `calls == transitions` is (`main.rs:2254`).
   - A `contended:<cause>` carry (`step_log_spike.rs:727`) took the slow path but has no tag. (a) cannot produce one, because its stream is new.
3. **Stale texts.**
   - "The harness and task T041-9 record the longest refill wait" (design:1269) contradicts design:2329-2330.
   - D041-120 point 7 still says "2.0% of (h)'s at both sizes", and its "corrected by D041-121" note does not mention that.
   - Complexity Tracking still says "six metrics and two alerts" (design:1923); with `fallbacks_total` and `ensure_affinity_misses_total` there are more.
4. **The start pools' setup leases are not declared.** `start()` prewarms its 4 pools of 16 with leases at i = 0..15 (`step_log_spike.rs:1872-1874`). The declaration's i = 20r + j covers only the replicas. Those 64 leases end at the close and are still counted in `lease_rows`. They have no effect on the gates.
5. **The prototype's ensure returns no `charged_at`.** It returns `(outcome, writer, room)` only (`step_log_spike.rs:382`), against data-model:671-672. That is acceptable for the spike, which has no store cache.
6. **Earlier nits 1, 3 and 4 remain open** (see the status above).
