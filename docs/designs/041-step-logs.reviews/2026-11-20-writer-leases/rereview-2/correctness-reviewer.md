# Correctness re-read of decision D041-120 (the ensure, the ended-backend reclaim, the fourth run's declaration)

Reviewer: `correctness-reviewer` subagent, read-only, on `9361a112`, reading the snapshot only. Paths are relative to the snapshot root `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot`.

**Summary.** There is one Blocker: as committed, the fourth run's harness stops every round in its first pool fill. There is one Major: the reading rule for gate (2b) can almost never pick the refill. Exactness and deadlock freedom hold on every changed path. The ensure never charges, and in the prototype it never raises for its listed outcomes.

## Exactness and deadlock freedom (checked, no finding)

**Exactness: 0 ≤ U ≤ S+G ≤ L and G = Σ granted on each path.**
- **`invalid`, `contended`, `room` and the lock-free `limit` answers.** None of them writes anything (`step_log_spike.rs:387-397`).
- **`made`.** This is the refill with `charge` false. The writer's lease is folded whole (S += used, G −= granted), then granted g ≤ F with `used = 0` (`:356-376`). So U is unchanged.
- **The ensure's `busy`, `held` and `limit` from the refill.** The block's rollback undoes the refill's writes and releases its row locks, so only the step-log fence, held shared, stays.
- **The lock-free `limit` matches the refill's own outcome.** At a refusal, F + Σ(others' room) = L − U. So the refill raises `held` exactly when g ≤ L − U and `exceeded` when g > L − U, and `L − U < n` implies `exceeded`.
- **Ended-backend reclaim** (`:429-456`). Under the usage row, each lease it locks `SKIP LOCKED` is folded (S += used, G −= granted) and deleted.
  - Pid reuse: suppose a new backend with the same pid charges the lease after the reclaim's cached `pg_stat_activity` snapshot. The EvalPlanQual recheck still qualifies the row, and the fold reads the locked, latest version, so its charge goes into S.
  - Suppose instead that backend holds the lease. Then `SKIP LOCKED` skips it. Either way the result is exact.
- **A carry with nothing to charge** (`:678`) charges nothing.

**Deadlock freedom.**
- The ensure's order is: step-log fence (tried, shared), then the usage row (bounded), then its own lease `NOWAIT`.
- The idle and ended reclaim's order is: fence (tried, shared), then the usage row, then leases `SKIP LOCKED`.
- The limit change takes the fence exclusively, so it waits for both of the above. Nothing waits after the usage row except the own-lease `INSERT` (earlier nit 1). I found no lock cycle.

## Status of my earlier findings (re-read of `fc4adff7`)

- **Major 1 (rule (ii) not computable): closed** by `carry_over_p99_by_path` (`main.rs:2272, 2350-2359`). The new Major 1 below concerns what the rule can conclude.
- **Major 2 (stale per-connection cache): mostly closed.** The cache is now keyed per (pid, tenant) and expires at 9 minutes. Batches and the marker return the room. T041-9 has a criterion on fallbacks per carry. Minor 5 below is what is left.
- **Major 3 (ensure failure): closed.**
  - The ensure returns a status and runs under the 200 ms budget.
  - The store never fails a report on it, and `limit`/`held` are passed to the carry.
  - Tests cover `invalid`, `contended`, `limit`, `busy`, `made` and `room`.
  - What is left is in minors 1, 2, 4 and 6.
- **minor 1 (held-room bound with ensure grants): partly closed.** The bound is restated, but there is still no test built from ensure grants, and the ensure's cap is loose (minor 8).
- **minor 2 ("at most 200 ms"; outer guard): partly closed.**
  - The outer guard is now stated (design:1228-1231, ADR-0097 decision 4), and ADR-0095 decision 5 names the `55P03`.
  - Still flat: FR-041-005 (design:162), design:1029 and ADR-0095:150.
- **minor 3 (lower `lock_timeout` untested): closed in code** by `a_callers_lower_lock_timeout_is_kept` and the 300 s replacement. The evidence is pending: per `handoff.md:52,56-57`, the 300 s change was never re-run and mutations 16 to 21 were never applied.
- **minor 4 (ensure test): mostly closed.** "An ensure charges nothing" (`leases_tests.rs:1534-1535`) and `contended` now bite. The `held` path and the refill-raised `limit` are untested (minor 6).
- **minor 5 (session affinity): closed** (gap G041-027, ADR-0097 decision 1, T041-9's pooler scenario).
- **minor 6 (lease table growth): closed in intent**, but the fold that closes it causes the Blocker below.
- **minor 7 (stale texts): mostly closed.** Still stale:
  - `step_log_spike.rs:260-268` still describes `sagat.step_log_charge_nowait`.
  - `step_log_leases_tests.rs:384-386` still says "only past it is a charge refused as held".
- **minor 8 (T041-9 criteria): partly closed.** G041-025 is restated to the cause it measures, which is fine. The new small-quota criterion cannot fail (minor 10).
- **Nit 1 (prepared transaction blocks the own-lease `INSERT`, unbounded): open.** T041-36's new "a prepared transaction holds the lease → busy" case covers only the `NOWAIT`.
- **Nit 2 (zero-byte carries): closed**, with a gap (minor 3).
- **Nit 3 (`caller="ingest"`): open.** It is renamed to "a batch's refill `55P03`", but a batch sets `lock_timeout` 200 ms for its whole transaction. Its stream-lock and fence waits raise the same SQLSTATE, so the store still cannot tell them apart.
- **Nit 4 (`a_completion_holding_the_usage_row_…` order): open.** It is still listed at design:1251.

## Blocker

**Blocker 1. The fourth run's pool fill asks the pool it has just emptied for a connection, so every round stops in its first row.**
- **Where:** `step_log_spike.rs:1756-1774`, against `tests/support/engine.rs:764`.
- **The code:**
  ```rust
  while held.len() < usize::try_from(max).unwrap_or(0) { … held.push(conn) … }
  …
  sqlx::query("SELECT spike_041.step_log_reclaim_idle($1)").bind(tenant)
      .execute(pool)   // every one of the pool's `max` connections is in `held`
      .await
      .unwrap_or_else(|error| panic!("fold the leases of ended backends: {error}"));
  ```
- **Scenario:** `SAGAT_PERF_STEP_LOG_CARRY_RUN=3`, so `PREWARM_LEASE` is set (`main.rs:2185`).
  1. The first measurement is off-1K at 1 × 16. `run_row` makes one replica with `API_POOL` = 20 and calls `prewarm`.
  2. `prewarm` holds all 20 connections, then `execute(pool)` waits for a 21st.
  3. After the 2 s `acquire_timeout`, `PoolTimedOut` makes it panic.
  4. Under the declared validity rule, "a round that stops is void". All 12 rounds of a 70 to 90 minute hosted run are void and the run is not decided.
- The diagnosis harness sets `PREWARM_LEASE` too (`main.rs:2433`), so `perf/041-diag-split` breaks the same way.
- **Why nothing caught it:** the carry spike is `#[ignore]` and gated by environment variables. No non-ignored test calls `prewarm` with a lease. The handoff's "50 tests passed" excludes the ignored benchmarks (`handoff.md:52`).
- **Smallest fix:** run the reclaim on a held connection (`.execute(&mut **held.first_mut().expect("a connection"))`) or before the fill. Also add a non-ignored test that:
  - calls `prewarm` with a lease on a 2-connection pool;
  - asserts each backend's lease has `granted = 1 MiB` and `used = u_i`;
  - asserts an ended backend's lease is folded.

  Pass the tenant as an argument rather than the global, so the test cannot race the benchmark.
- **Confidence:** confirmed in code.

## Major

**Major 1. The (2b) reading rule cannot attribute a failure to the refill under the run's own expected refill share.**
- **Where:**
  - design:2287-2289 (the expectation: "the refills being too few to fill its top 1%");
  - design:2297-2300 (the rule: "above one half, the lever is the refill");
  - `main.rs:2350-2359`.
- **The arithmetic, for (a) with 9,000 carries.**
  - `over_p99_by_path` counts carries strictly above `times[8910]`, at most 89 carries.
  - Expected refills are 28.2. Even if all of them are in the tail, the share is 28/89 = 31.5%.
  - Adding the up-to-9 mark-only answers that a valid round tolerates (0.1%) gives 37/89 = 41.6%.
  - Reaching one half needs at least 45 refill or mark-only carries above p99: 1.6 times the expected refills, every one of them slower than p99.
- **Consequence:** the rule as declared nearly always answers "the lines' p99" (section 3's chunk rows, D041-111).
  - Yet 28 slow refill carries can still raise (a)'s p99 by moving 28 of the roughly 90 tail positions. That matters for a gate the declaration calls "close … may go either way" (+56 against about 61 ms).
  - At 1 × 16, (2b) is expected to fail "for the carry's own cost", the same reason given for (1) and (3). Those two go to the environment question, but the rule sends (2b) to a design lever.
- **Why nothing caught it:** the rule is computable now (my earlier Major 1), but it was never checked against the expected shares the same declaration states.
- **Smallest fix:** declare a counterfactual rule before any round.
  - Tag each completion with its carry's path (the hook knows it).
  - Print (a)'s completion p99, and the carry p99, with refill and mark-only completions excluded.
  - If (2b) passes without them, the lever is the refill; otherwise the lines.
  - Treat a (2b) failure at 1 × 16 like (1) and (3) there.
- **Confidence:** confirmed by arithmetic against the code.

## minor

**minor 1. "So a report waits for the usage row at most once" is false.**
- **Where:** design:1205-1207; ADR-0097:122-125; research.md D041-120 point (1).
- **Scenario:** a release holds the usage row for 500 rows.
  1. The ensure waits 200 ms and answers `busy`. The store does not pass `busy` to the carry.
  2. The reserve finds no room, so the fallback waits up to 200 ms again, this time under the run lock, and most likely answers `mark_only:busy`.
- The same double wait happens after `made` when a near-limit reclaim takes the room before the report.
- **Why nothing caught it:** T041-36's planned case ("an ensure … past its budget answers `busy` and the report lands", design:2690-2693) does not say what the carry then does.
- **Fix:** pass `busy` to the carry as well, so it answers `tenant_quota_busy` at once with no wait under the run lock, or restate the bound as two waits.
- **Confidence:** confirmed in documents.

**minor 2. The ensure's lock-free `limit` ignores `step_log_enforcement_enabled`.**
- **Where:** data-model:269 against :232-233 and :262.
- **Scenario:** enforcement is off and U > L after a lowered limit. The charge and the refill must not refuse. Yet whenever the cached room is short, the ensure answers `limit`, the store passes `tenant_quota`, and the records are dropped.
- **Fix:** answer `limit` only when enforcement is on, as the reserve already does (data-model:285).
- **Confidence:** confirmed in documents. The prototype has no flag.

**minor 3. "Nothing to charge" is defined as "no record kept", but a carry can keep no record and still insert a ceiling marker.**
- **Where:** data-model:582 and `step_log_spike.rs:678` (`v_kept = 0`), against data-model:554-555 and design:1018 ("one ceiling marker planned when a ceiling is first met").
- **Scenario:** the carry's first record does not fit the attempt ceiling.
  1. The plan keeps 0 records and one marker, which is 128 bytes plus its text.
  2. Because the condition is "no record kept", there is no reserve and no block.
  3. If the lease is short, CD's refill runs unprotected. A `55P03` or `tenant_step_log_quota_exceeded` raises out of the call, and the report is re-run without the carry, its records counted `failed`.
- This contradicts "the quota's own refusals never reach this path" (design:1093).
- **Fix:** skip the block only when the planned charge is 0, meaning no record and no marker. Add a T041-36 case of a carry whose only row is a marker, with a short lease.
- **Confidence:** confirmed in documents.

**minor 4. Refusals that come through the ensure lose their cause, so the held alert under-counts.**
- **Where:** data-model:512-513 and :519; design:1313 and :1768-1770.
- **The gap:** with `p_mark_reason = 'tenant_quota'` the function cannot tell `limit` from `held`. Yet `step_log_quota_refusals_total{cause}` is "counted by the store from the carry's returned cause".
- **Consequence:** at a held tenant most refusals come through the ensure (the cached room is short). So `SagatStepLogQuotaHeldRefusals` sees only the fallbacks that came after a reclaim.
- **Fix:** have the store count `refusals_total{cause}` from the ensure answer it passes on, or add a cause argument that the carry echoes.
- **Confidence:** confirmed in documents.

**minor 5. The store's 9-minute expiry starts when it saw the room, not when the lease was last charged.**
- **Where:** `step_log_spike.rs:392-395` (`room` writes nothing, so `charged_at` stays put); ADR-0097:125-127; design:1207-1209 and :1278.
- **Scenario:**
  1. At t = 0 a carry charges the lease, so `charged_at` = 0.
  2. At t = 9.5 minutes a report with a secret writes a mark-only carry. It returns the room, and the store refreshes its entry. The same happens with a carry answered `contended`, or with a report rolled back after its carry.
  3. At t = 10.1 the idle reclaim deletes the lease.
  4. At t = 12 the store's entry is 2.5 minutes old and covers the planned charge, so no ensure runs. The carry takes the fallback under the run lock.
- **Fix:** return the seconds left before the lease goes idle with the room, and expire the entry on that. Also update the cache only after the report commits.
- **Confidence:** confirmed in code; impact plausible.

**minor 6. The ensure's `held` answer, and its `limit` raised by the refill, are untested.**
- **Where:** `step_log_leases_tests.rs:1738-1775`.
- **Gap:** a mutation that drops the `WHEN program_limit_exceeded` handler (`step_log_spike.rs:404-409`) survives every test, yet `held` is the answer passed to the carry. Both cases are reachable: for example L = 100,000, writer A holding 60,000 of room in an open transaction, and an ensure of 50,000 on writer B answers `held`.
- **A second gap:** the `again` assertion in `the_ensure_before_a_report_gives_the_carry_its_fast_path` (:1549-1559) can no longer fail.
  - With the bounded ensure, mutation 14 (no room check) answers `busy` within 200 ms, which still matches `Ok(Ok(_))`.
  - Mutation 14 is now caught only by the new test's final `room` assertion, so validation-report:800 is stale.
- **Fix:** add the held case and a stale-snapshot `limit` case, and assert that `again` answers `room`.
- **Confidence:** confirmed in code.

**minor 7. The ensure opens a subtransaction that is neither documented nor needed.**
- **Where:**
  - `step_log_spike.rs:399-410`: every `made` ensure writes inside a `BEGIN … EXCEPTION` block, so it is assigned a subtransaction XID.
  - ADR-0097:278: "the refill and a carry with nothing to charge open none".
  - design:1116: the budget row does not mention it.
- **Consequence:** in T041-9's many-low-volume-tenants regime, ensures are the main refill path, so the subtransaction rate the criterion judges comes mostly from them.
- **Fix:** drop the block. The ensure is its own transaction, so the store can roll it back and map `55P03` or the constraint name to the answer. Otherwise, document the subtransaction in the budget row and in T041-9.
- **Confidence:** confirmed in code.

**minor 8. The ensure's `invalid` cap is an attempt's maximum, not a carry's.**
- **Where:** `step_log_spike.rs:385-388` and data-model:266-267 (29,577,344 bytes), against design:1201 ("larger than any valid carry's") and C2 (design:964: at most 64 records and 16 KiB encoded, so roughly 16,384 + 128 × 65 = 24,704 bytes plus normalization growth).
- **Consequence:** a runtime-role call can lease up to about 28 MiB past the share. The restated held-room bound ("shares plus reserved planned charges", design:1256-1259) then rests only on the store's discipline.
- **Fix:** cap at the carry's maximum planned charge. Add the ensure-grant case asked for in my earlier minor 1.
- **Confidence:** confirmed in documents and code.

**minor 9. The ensure's delay comes before the report's fence check, so it can push a report past its lease.**
- **Where:** design:158-165 (FR-041-005's residual names only the downgrade re-run) against :1196-1200.
- **Scenario:** a step reports close to its lease expiry. The ensure waits about 200 to 400 ms, or up to 5 s under the statement timeout. The report then arrives late and is refused, which changes the step's answer.
- **Fix:** skip the ensure when the attempt's lease ends within the statement timeout, or list this case under G041-021.
- **Confidence:** plausible.

**minor 10. Two of T041-9's new criteria cannot fail or are not grounded.**
- **Where:** design:2943-2949.
- **Small quota: "every held refusal comes with unleased room under its charge and held room within the operator's bound".** Both halves follow from the refill's arithmetic (`step_log_spike.rs:360-370`):
  - g = max(n, share) with share ≤ F/2, so g > F implies n > F;
  - each lease's room is at most max(n, 1 MiB).
- **Low volume: "under 1,000 a second (the rate below which `pg_subtrans` stays in its default buffers)".** The pages `pg_subtrans` needs depend on the oldest snapshot's XID span, not on a rate. Workload (d) holds a transaction open on purpose.
- **Fix:** judge held refusals by their rate and by the held room as a share of L; state the subtransaction criterion as SLRU misses per second measured from `pg_stat_slru`.
- **Confidence:** confirmed by arithmetic (small quota); plausible (low volume).

**minor 11. Stale or overstated texts remain.**
- "At most 200 ms", stated flat: FR-041-005 (design:162), design:1029 and ADR-0095:150.
- `step_log_spike.rs:260-268` and `step_log_leases_tests.rs:384-386` (see the status of earlier minor 7).
- "The harness … record[s] the longest refill wait" (design:1231): the harness records the longest refill-written carry time (`main.rs:2273`), not the refill's wait.
- G041-025's closing rule gives expected shares only for (a) and (h). Workloads (f), (g) and (d) also carry, at 0.31%, and the rule does not say so.
- "2.0% of (h)'s at both sizes" (design:2287-2288; D041-120 point 7): (h) runs only at 1 × 16 (`main.rs:2332-2335`).
- **Confidence:** confirmed.

## Fourth-run declaration against the prototype and harness (checked)

- **Setup and expected shares (design:2259-2275) match the code** (`step_log_spike.rs:1770-1795`): the fold, the 1 MiB lease and the charge of u.
- **L is the catalogue's maximum, 2^50** (`quota.rs:28`), so every grant is 1 MiB and nothing is refused or held.
- **The share formula.** 2,654,435,761 mod 2^20 = 489,905, which is odd, so u_i = ((i+1) × 489,905) mod 2^20 is never 0. The 20 fill levels of a pool average 0.506 MiB.
- **Arithmetic for (a):**
  - 9,000 × 3,280 = 29,520,000 bytes;
  - ÷ 1,048,576 = 28.15 refills, 0.313% of the carries;
  - the per-grant cycle (1 refill plus 318 fast carries) gives 1/319 = 0.313%;
  - a deterministic count at 1 × 16 (20 backends, about 1.41 MiB each) gives 28.
- **Arithmetic for (h):**
  - 64 × 328 = 20,992 bytes per carry; × 9,000 = 188,928,000;
  - ÷ 1,048,576 = 180.18 refills, 2.002%;
  - the per-grant cycle (1 + 48 carries) gives 2.04%.
- **The reading rules:**
  - The validity counts, G041-025's closing rule and the "void only by busy" rule can all be applied from `spike-carry-json` and `carry_spike_judge.py:29-31`.
  - The (2b) rule can be computed, but see Major 1.

## nit

1. **The p99 index is off by one.** `over_p99_by_path` uses `times[n·99/100]` (`main.rs:2352`). The printed p99 uses `ceil(n·0.99) − 1` (`main.rs:301`), so the threshold is one element above the printed p99.
2. **"About a fifth of its lease"** (design:2274-2275): at 8 × 128 there are 8 × 20 = 160 backends, and 29,520,000 / (160 × 1,048,576) = 0.176. That is nearer a sixth.
3. **Where *i* is counted.** The declaration says "*i* the connection's index in its row's pool fill" (design:2266). The code counts per pool (`step_log_spike.rs:1768-1775`), so every replica pool gets the same 20 fill levels.
4. **`lease_rows` is not filtered by tenant** (`step_log_spike.rs:2289`). It also counts the 64 leases of the start pools, which have ended but are not yet folded.
5. **One count, two labels.** `sagat_quota_step_log_reclaim_idle` returns one count, yet data-model:281 feeds both `{kind="idle"}` and `{kind="ended"}` from it.
6. **Earlier nits 1, 3 and 4** remain open (see the status above).
