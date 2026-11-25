Doc: docs/designs/041-step-logs.md
Reviewed-commit: 543417134ff7ebda70a96690329bb95ddde55c8c
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888

**Verdict: approvable on correctness once one Major is fixed or knowingly accepted.**
- All of my third-round findings are resolved: B-1, the N-2 remainder, M-1 to M-4 and Nits 1 to 6. Two stale passages remain (Nits 1 and 2 below).
- Every number I was asked to check is right.
- The fixes add one new Major. The owner-report threshold for the p99 repeats B-1's error: it compares the median of all valid rounds against the largest of the three rounds used. At 8 × 128 it fires on the accepted build's own recorded rounds.
- Caveat: I read the worktree from disk and could not confirm that HEAD is the reviewed commit (`git -C <worktree> rev-parse HEAD`).

### Third-round findings

- **B-1: resolved.**
  - Design: `docs/designs/041-step-logs.md:430-435`; T041-9 at `:3443-3445`.
  - Also `specs/041-step-logs/research.md:147` (D041-136), `docs/owner-decisions.md` OD-2026-11-21-1, `docs/runbooks/step-logs.md:35-40` and `specs/041-step-logs/validation-report.md:780-783`.
  - Checked: the rounds used have median −17.03. The worst valid round is +112.86 (round 6), so the tolerance is 129.89 and the bar +112.9, rounded in the build's favour.
- **N-2 remainder: resolved.**
  - At least seven valid rounds, the even-count median and the rule for fewer than seven: `:425-429`.
  - The rate for each m and the per-release figures: `:446-457`.
  - Checked: 35/1,365 = 2.56%, 70/1,820 = 3.85%, 126/6,188 = 2.04%, 252/8,568 = 2.94%, 462/27,132 = 1.70%, 924/38,760 = 2.38%.
  - The formula is exact for odd m and an upper bound for even m, as stated.
  - The resampling estimates, by exact enumeration with p = 2/8 below 90%: gate (1) 7.06% (m = 7), 7.36% (m = 8), 4.11% (m = 12); gate (3) 7.06%, 7.47%, 4.12%.
  - The union bounds: 5 × 3.846 + 7.36 + 7.47 = 34.06%, and 5 × 2.384 + 4.11 + 4.12 = 20.15%.
- **M-1: resolved.**
  - `:1473-1485` and T041-36 at `:2899-2920`; the fault test at `:3057-3067`.
  - sqlx-core 0.9.0 `pool/connection.rs:97-99` confirms the close flag can only be set. `:314` confirms that `return_to_pool` pings, which drains a statement in flight, so "sqlx first drains" holds.
  - No helper in `crates/infra-postgres/src` nests sqlx `begin()` (only raw `SAVEPOINT` in `events.rs:480-536`), so sending `BEGIN` directly breaks no savepoint.
- **M-2: resolved, with a residue (Nit 3).**
  - `limit` is tested before `room`, and `held` is gone: `:1489-1493`, `specs/041-step-logs/data-model.md:320` and `:506`, ADR-0097 `:137-142` and `:220`, T041-5 at `:2611-2614`.
- **M-3: resolved.**
  - `:1426-1431`, the risk row at `:2126`, ADR-0097 `:333-337`, T041-37 at `:3180-3187` and the exclusion at `:3197-3201`.
- **M-4: resolved.**
  - The bound is stated for a replica's own pool, with run-row queues outside it: `:1414-1421`.
  - Pooler sizing: `:1422-1425`. The victim tenant in the fan-out regime: `:3192-3196`.
  - *Q* is passed per transaction: `:1404-1411`, data-model `:251-257`. The `options` handling: `:3137-3144`.
- **Nits 1 to 6:**
  - Nit 1 (stale top-up text) is resolved at design `:1295` and data-model `:247-248`.
  - Nit 2 (which p99 statistic) is resolved, and its new threshold is the Major below.
  - Nit 3: the test at `:3040-3043` can now fail when the replica-slot mutation lets five waiters fill a pool of 8.
  - Nit 4 is resolved in text at data-model `:259-264` and `:317-318`, but its test cannot fail (Nit 4 below).
  - Nit 5 is resolved at data-model `:474-483` and the runbook `:107-108`.
  - Nit 6 is resolved in `resolution.md:324-328`.

### Numbers checked

- **The 8 × 128 medians over the eight valid rounds:**
  - (1): (0.9053 + 0.9306) / 2 = 0.91795.
  - (2b): (−2.19 + 6.30) / 2 = +2.06 ms.
  - (3): (0.9154 + 0.9228) / 2 = 0.9191.
  - (4): 5 tickets.
- **The rounds that missed 90% at 8 × 128:** 89.58% and 88.63% for (1); 83.39% and 89.36% for (3). Both pairs are right.
- **The admitted p99 at 8 × 128:** 481.5 + 48.15 + 112.9 = 642.55 ms, and 642.55 / 481.5 = +33.4%.
- **Pool figures:**
  - At 20: *Q* 5, *P* 3, cap 10, 5 connections left.
  - At 10: 2, 1, 5, 3 left, tickets min(7, 5) = 5.
  - At 8: 2, 1, 4, 2 left.
  - At 3: 1 left. At 2 or less: none.
- **Server pool:** 8 × (10 + 5) + 8 = 128. The benchmark database runs with `max_connections=400` (`tests/performance/reports/scalability-ha.md:70`).
- **"Below a pool of 16":** for N ≤ 15 the cap ⌊N/2⌋ is at most 7, so min(7, cap) equals the cap; at 16 it does not (7 < 8). Right.

### New findings

**1. Major (fix). The owner report on the p99 fires on the accepted build.**
- **Where:** `docs/designs/041-step-logs.md:459-464`, T041-9 at `:3448-3452`, `docs/runbooks/step-logs.md:39-41`, `research.md:147`.
- **The rule:** the median over the valid rounds of (a)'s p99 percent is compared with the largest value among the three rounds used (+9.5% at 8 × 128, +36.9% at 1 × 16).
- **Why it fires:** with the allowance at 10% (`validation-report.md:758`), percent = 10% + excess / p99(off-1K).
  - Round 2: −2.19 / 482 gives +9.55%, which is the recorded "+9.5%".
  - Rounds 6, 7, 9 and 11 have a positive excess, so they are above +10%. Round 4 is far below.
  - So the eight valid rounds' median is at least (9.55 + 10) / 2 = 9.77%, above the threshold. The build the owner accepted triggers the report with its own figures.
  - At 1 × 16 the eight-round median is about +36.1%, just under the bar. But rounds 6, 9 and 11 are around +85% to +94%, and round 2 is at +36.96%. A new run of 7 to 12 rounds therefore exceeds the bar roughly 25% to 50% of the time.
- **Why it matters:** the report restates the runbook's and the release notes' figures and goes to the owner. So the signal is close to a coin flip for an unchanged build.
- **Fix:** take the threshold from the same statistic of the fourth run, the eight valid rounds' median of that percent (computed from `verdict.txt` and recorded in `validation-report.md`). Or state that the threshold sits at the worst valid round.
- **Confidence:** confirmed from the recorded values at 8 × 128; plausible at 1 × 16, because the per-round off-1K p99 of rounds 4 to 11 is not in the table.

**Nits**
1. **(fix)** `docs/designs/041-step-logs.md:1590` still lists `"held"` among the top-up's outcomes. That contradicts ADR-0097 `:220` and data-model `:506`.
2. **(fix)** data-model `:705` (§8a) still says the fallback tries the usage row `NOWAIT`. Inside the carry's block, a literal `NOWAIT` would raise `55P03` into the block's handler and answer `service_busy` without taking a slot or waiting. Write `SKIP LOCKED`, as §3b does.
3. **(fix)** After a limit is lowered below S, or with enforcement off, F = L − S − G can be negative. Then ⌊F / (2(A + 1))⌋ < 0, so "`limit` when the share is 0" (data-model `:320`, design `:1489`) falls through to `room`: a skipped case counted as success again. Say "0 or less" and add a test with F < 0.
4. **(fix)** T041-5's check that `pg_stat_get_backend_subxact` reads 0 while the refill waits (`:2608-2609`) cannot fail for the mutation it guards. A caught `NOWAIT` block has already aborted its subtransaction by the time the refill waits, and PostgreSQL drops an aborted subtransaction's XID from the backend's cache. Use a source-text check that the refill and top-up bodies contain no `EXCEPTION` (T041-5 already has migration source checks).
5. **(fix)** The "no runtime current" branch (`:1482`, T041-36 `:2904-2906`) marks the connection `close_on_drop()` and drops it. In sqlx-core 0.9.0, `PoolConnection::drop` spawns on both paths (`pool/connection.rs:199-210`), and `rt::spawn` panics without a Tokio context (`rt/mod.rs:70-91`, `:164-166`). Use `detach()` there, or state that this branch is unreachable.
6. **(fix)** "B's `maxwait`" (`:3185`) cannot be measured: `SHOW POOLS` reports per database and user, and the benchmark uses one role. Judge the pool's `maxwait`, which is stricter, or B's acquire time on the client side.
   - Also state that at the rule's minimum the pooler reserve is R = 8 backends shared by all replicas' ordinary work, not 5 of 20 per replica.
   - `docs/runbooks/scaling.md:36` should cite the sizing rule.
7. **(fix)** "At most 34.1%" and "at most 20.1%" (`:454-457`, `:3448`) each add two resampling estimates, so they are estimates, not bounds.
   - OD-2026-11-21-1 cites only the per-gate 3.85%. About 15 points of the per-release figure come from gates (1) and (3) at 8 × 128, which the owner was not asked about.
   - Say "about", and put the per-release figure in the owner row.
8. **(accept)** Counting only on commit (data-model `:509-511`) leaves out a carry whose `COMMIT` outcome is unknown (cancelled in flight) but which did commit; its replay stores nothing. The metrics undercount in that rare case, while the stream row stays exact. State it.

Counts: Blocker 0, Major 1, Nit 8