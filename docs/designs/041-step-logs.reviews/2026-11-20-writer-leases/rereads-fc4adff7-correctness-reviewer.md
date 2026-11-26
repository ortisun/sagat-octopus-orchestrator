# Correctness re-read of decision D041-119 (writer leases, the ensure, the 200 ms fallback)

Reviewer: `correctness-reviewer` subagent, read-only, on `fc4adff7`. Saved verbatim.

**Re-review of the writer-owned leases at fc4adff7 (design 041, ADR-0097, ADR-0095 decision 5, D041-119)**

There is no Blocker. Exactness and deadlock freedom hold on every path in the prototype. I found three new Majors: the fourth run cannot apply one of its own reading rules, the store's choice of when to run the ensure goes stale, and nothing says what happens when the ensure fails. Paths below are relative to `/Users/marcelo/Documents/Projects/sagat-wt-041`.

## Status of my earlier findings

- **Major 1 (the bound *b*):** closed by the redesign. A carry that waits past the budget now gets a `busy` mark-only answer and its report lands. The harness counts `mark_only:busy`, and a carry that raises makes `calls ≠ transitions`, so its `assert_eq!` stops and voids the round (`main.rs:2254`).
- **Major 2 (held-room bound):** closed for a charge whose bytes are within its share. It is not closed for the ensure path (minor 1).
- **Minor 3 (idle reclaim and limit change):** closed. Its test bites under mutation 11.
- **Minor 4 (tests that cannot fail):** partly closed. The fence wait is now filtered by database, the formula test is gone, the default budget is exercised, and the restore is tested. Still open: "a lower `lock_timeout` is kept" (minor 3), and the non-production order of `a_completion_holding_the_usage_row…` (nit 4).
- **Minor 5 (total wait):** documented, with stale "at most 200 ms" wording left elsewhere (minor 2).
- **Minor 6 (counter mixes ingests):** closed, with nit 3.
- **Minor 7 (drift):** mostly closed; what is left is in minor 7.
- **Minors 8, 9 and 10:** closed.
- **Nit 11 (prepared transactions):** partly closed (nit 1).
- **Nit 12 (session setting):** closed.

## Exactness and deadlock freedom (checked, no finding)

**Exactness.** 0 ≤ U ≤ S+G ≤ L and G = Σ granted hold on each path:
- the fast charge, under the fence shared, reading S+G ≤ L;
- the refill with charge true or false (`step_log_spike.rs:356-376`), folding the own lease whole and granting only g ≤ F;
- the near-limit reclaim (`:339-345`): S += used, G −= granted, with U unchanged;
- the idle reclaim (`:402-411`);
- the limit change (`:484-490`), under the fence held exclusively;
- the release: under the usage row, any concurrent fast charges only raise U.

A refusal in the carry's block rolls back its reclaim and fold together.

**Deadlock freedom.** The usage row is taken after the step-log fence everywhere: refill, ensure, release and idle reclaim. The only lease wait is the limit change's `NOWAIT`, which runs under the fence held exclusively, and the idle reclaim now tries that fence first. After the usage row a refill takes only its own lease `NOWAIT`. I found no lock cycle. The one caveat is nit 1.

## Major

**Major 1. Reading rule (ii) of the fourth run cannot be computed from what the harness prints.**
- Where: design 041:2233-2235 against `tests/performance/scalability-ha/main.rs:2263-2278`.
- Scenario: (2b) fails at 8 × 128, which the declaration itself says "may go either way". Rule (ii) then needs "the share of `written:refill` carries among the carries slower than the row's carry p99".
- What exists: the gate harness's `spike-carry-json` prints only the overall `carry_p50`/`carry_p99` and the counts. `times_by_path` is printed only by the diagnosis harness (`main.rs:2549-2567`), and even there `carry_over_p99` is not split by path.
- Consequence: the declared rule cannot be applied. The attribution would be made after the fact, or the hosted runs repeated.
- Why nothing caught it: no test reads the JSON line.
- Fix: for each row, emit the count of carries above the row's carry p99 by path (fast, refill, mark), or the sorted `times_by_path`. Do this before any round.
- Confidence: confirmed in code.

**Major 2. The store decides whether to run the ensure from a per-connection cache that several paths make stale, so the "rare" fallback is not rare for low-volume tenants or long steps.**
- Where: design 041:1181-1186, 1244 and 2565-2568; ADR-0097 decision 3b (`ADR-0097…md:100-108`).
- The rule: the store compares the planned charge with "the room the previous carry on that connection returned".
- Stale source 1 — the idle reclaim. Scenario: a backend carries for tenant T at t=0 and has 500 KiB of room left. It serves other tenants for 11 minutes, and the idle reclaim deletes T's lease. T's next report on that connection sees 500 KiB cached, so it skips the ensure. The reserve finds no lease, and the fallback refill runs under the run lock with its subtransaction.
- That contradicts 1244 ("a live writer charging less often pays one ensure to lease again"). For "many low-volume tenants" this becomes the normal path.
- Stale source 2 — ingest batches on the same pool. Their charges use up the same backend's lease, so the cached room is too high in (g) and (h). Nothing in the design mentions this.
- The ensure's own early return (`step_log_spike.rs:385-388`) can also return for a lease that is about to go idle.
- Why nothing caught it: the fourth run has no ensure. T041-9's low-volume criterion (2862-2864) judges only the subtransaction rate, not fallbacks under the run lock.
- Fix: expire a cache entry after the idle interval minus a margin; have batches update the same per-(connection, tenant) cache. Or call the ensure on every carrying report — the test shows it takes no row lock when there is room. Add a T041-9 criterion on fallbacks per carry.
- Confidence: confirmed in documents; impact plausible.

**Major 3. Nothing says what happens when the ensure fails, and at the limit it always fails.**
- Where: data-model §3b:261-264; design 2b:1181-1188 and T041-36:2565-2572; FR-041-005 (design:153-154).
- The ensure is a refill with charge false, so it raises `tenant_step_log_quota_exceeded` or `held` when g > F.
- Scenario: a tenant at its quota, which is an ordinary state for a lossy log. The previous carry returned room 0, so every carrying report first runs an ensure. Each ensure locks the usage row, runs the near-limit reclaim, raises and rolls back.
- Nothing says the store swallows that error and goes on with the report. If it propagates, every report of a full tenant fails for a step-log reason, against ADR-0084 decision 7.
- Even when it is swallowed, it rebuilds a usage-row queue of failing transactions at exactly the regime ADR-0097 set out to relieve.
- The ensure also has no wait budget. Design 1185 bounds it only by the statement timeout (5 s), which goes into the step's completion time without a bound FR-041-005 states.
- The planned tests (2571-2572) cover none of this.
- Fix:
  - the ensure returns a status (made, limit, held, busy) instead of raising;
  - it waits under its own `lock_timeout`;
  - the store never fails a report on it;
  - the store skips the ensure for a tenant whose last carry was refused for its limit, until a release or a limit change;
  - add tests: an ensure refused at the limit, then the report lands with `mark_only:quota`; an ensure that meets a held row past its budget, then the report lands.
- Confidence: confirmed in documents.

## minor

**minor 1. The held-room bound does not cover leases the ensure made, and its test pins only 1-byte charges.**
- Where: ADR-0097:131-137; design 041:1227-1235; `step_log_spike.rs:360, 372, 389`.
- The derivation's premise is "a grant past the charge takes at most F/(2(A+1))". That holds when the charge n ≤ the share. The ensure grants max(n, share) and charges nothing (`used = 0`), so between a report's reserve and its CD the lease holds up to n beyond its share.
- Scenario: L = 1 MiB, F₀ = 65,536. Five backends each ensure 8,192 and reserve at once. F goes 32,768 → 24,576 → 16,384 → 8,192 → 0, against the stated floor F₀·C(10,5)/4⁵ = 16,128. A 3,280-byte carry on a sixth backend is then refused `held`.
- The parenthetical "with 128 writers holding, as little as about F₀/20" sits after the general case, where no such floor exists.
- Why the test misses it: `room_held_by_open_writers_is_bounded_from_the_free_room` uses n = 1 charges only, with no ensure.
- Fix: state the bound as the unused room of charge grants, plus the planned charges of carries between their reserve and their CD. Move the parenthetical. Add a case built from ensure grants with n > share.
- Confidence: confirmed by arithmetic against the code.

**minor 2. "At most 200 ms" overstated, and the outer guard's outcome not stated.**
- Where it is overstated: ADR-0097 Consequences:234-235, ADR-0095:145 and FR-041-005 (design:153-154) say the fallback waits "at most 200 ms".
- Why that is wrong: decision 4 and 2b say `lock_timeout` applies to each acquisition, so a wait can exceed it.
- ADR-0095:86-87 still says the block "catches only those two refusals". Lines 93-95 add `55P03`.
- The outer guard's outcome is not stated next to the budget: a `57014` in the carry gives `StepLogCarry`, then a re-run with no carry (records `failed`), with the run row held up to 5 s.
- Fix: align the wording, and add that sentence to 2b.
- Confidence: confirmed in documents.

**minor 3. "A caller's lower `lock_timeout` is kept" is untested.**
- Where: `step_log_leases_tests.rs:1479-1508`; the code is `step_log_spike.rs:324`.
- With 150 ms and no contention, the refill sets nothing visible and the restore hides any change.
- A mutation that always sets *b*, or that inverts the comparison, passes all 30 tests, and mutation 13 does not cover it.
- The busy test also passes with a 30 s budget, because its bound is 60 s.
- Fix: set the caller's `lock_timeout` to 50 ms, replace the wait function with 60 s, hold the row, and expect `mark_only:busy` (no timing assertion is needed). Assert the wait function returns 200 ms.
- Confidence: confirmed in code.

**minor 4. The ensure test does not pin exactness, the fence or the failure path.**
- Where: `step_log_leases_tests.rs:1510-1551`.
- A mutation where the ensure charges its bytes (`used = p_bytes`) survives: the carry still takes the fast path, and `assert_exact` holds while U overcounts by 3,280.
- A mutation where the ensure takes no fence also survives: a concurrent limit change would then fail on `NOWAIT`.
- Fix: assert S + Σ used is unchanged across an ensure. Run a limit change while an ensure holds its lease.
- Confidence: confirmed in code.

**minor 5. Decision 3b requires the same backend, which decision 1 says need not hold.**
- Where: ADR-0097:71-72 against 103-104.
- Decision 1 claims correctness "behind a transaction pooler too".
- Decision 3b relies on "the same connection, so the same backend" for two transactions in a row. Under transaction pooling the ensure leases backend X, the report runs on Y, and every refill becomes a fallback.
- Fix: state that session pooling is required, or have the carry return `pg_backend_pid()` and key the cache by it.
- Confidence: confirmed in documents.

**minor 6. The fourth run's lease table grows across rows.**
- Where: `step_log_spike.rs:1718-1727`, called for the start pools at `:1761`; `:624` and `:358-359`.
- `prewarm` leases every pool, including the four throwaway start pools (64 connections). The pools close after each row and no idle reclaim runs, so dead-pid leases build up: about 84 per row at 1 × 16 and 224 per row at 8 × 128, roughly 5,500 after three rounds.
- Every carry runs `step_log_usage` (a sum over all of the tenant's leases) and every refill counts them. So the carry's cost rises from round to round, unlike the built design, whose idle reclaim bounds the rows.
- The gate harness does not print `leases_before`/`leases_after`.
- Fix: before each row, fold and delete leases whose writer is not in `pg_stat_activity`; do not lease the start pools; print the lease count per row; or declare this in the run's text.
- Confidence: plausible.

**minor 7. Documents and comments that are out of date.**
- T041-5 (design:2383-2384) does not exempt `sagat_quota_step_log_ensure` and `…_reclaim_idle` from "not executable by `sagat_app`". Data-model §4:339-340 grants both, so the privilege test as specified would fail, or the store could not call the ensure.
- G041-023 (research.md:158) still says the grant is "n − room" and that a refill past its bound "fails the report, which is re-sent".
- G041-025 (:160) still cites D041-117/118, "bounded and loud".
- Design:1121-1123 pins the carry's nested statements "as 23"; 2b and the test say 26.
- §8a:500-501's `dropped_by_server` reasons leave out `tenant_quota_busy`.
- `step_log_spike.rs:260-268` still describes `sagat.step_log_charge_nowait`.
- `step_log_leases_tests.rs:384-387` repeats the refuted "L·C(2k,k)/4^k … only past it is a charge refused as held".
- Fix: correct each text.
- Confidence: confirmed.

**minor 8. Two T041-9 / G041-025 criteria cannot be evaluated as written.**
- T041-9's small-quota criterion (design:2860-2861), "held refusals only while the free room is under the bound of section 2b": that bound needs F₀ and *k* consecutive grants, which concurrent charging from 1,024 backends does not have. Fix: measure the held room at each refusal against Σ min(1 MiB, share) over the held leases.
- G041-025 is closed by the fourth run alone (design:2231-2232; research:160). There the setup lease stands in for the ensure, so the defect's production path, the built ensure in a cold start, is tested only by T041-9. Fix: make T041-9's cold-start result a closing condition too.
- Confidence: confirmed in documents.

## nit

1. **Prepared transactions** (`step_log_spike.rs:350-351`). When the own lease is invisible (`NOT FOUND`), the `INSERT` can wait on a prepared transaction's uncommitted insert of the same key. That wait holds the usage row under the caller's restored `lock_timeout`, which is 0 for a report. The resolution's "Not taken" claims `NOWAIT` covers prepared transactions; it does not cover this insert. Fix: document it, or check at startup that `max_prepared_transactions = 0`.
2. **Zero-byte carries.** A carry whose planned charge is 0 (any mark-only carry, or nothing kept) always takes the block, because the reserve returns false for bytes ≤ 0 (`:422`). That contradicts "a subtransaction only on the fallback path" (ADR-0095:143-144). Fix: skip the block when nothing will be inserted.
3. **`caller="ingest"` counts too much.** It counts every mapped `55P03` of a batch (stream lock, fences), not only refills. Fix: name the label for what it counts.
4. **`a_completion_holding_the_usage_row_refills_while_another_refill_waits`** (`step_log_leases_tests.rs:1228-1261`) still stages a content update before the carry, an order production never produces. It is still listed as a deadlock case in design:1222.
