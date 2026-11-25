**Correctness re-read 3 of design 041 (approval run, at `c52b5d37`, files read without a shell)**

**Verdict: not approvable yet.** N-1, N-3 and N-5 are resolved. N-4 is resolved in the normative text, with two stale passages left. N-2 is only partly resolved. Reading the newly recorded eight valid rounds against the new "median of all valid rounds" rule turns up a new Blocker at 8 × 128. Four new Majors come from the second-round fixes. Caveat: I read the worktree as it is on disk. I could not confirm that HEAD is `c52b5d37` (`git -C <worktree> rev-parse HEAD`).

### Second-round findings

- **N-1 (`failed` missing): resolved.**
  - `failed` is in the mark-reason set in `specs/041-step-logs/data-model.md:19`, `:609` and `:634`.
  - The design agrees at `docs/designs/041-step-logs.md:1114`, `:1136`, `:1154`, `:1231-1235` and `:2799`.
  - T041-36's re-run test is split at `:2941-2947`, and "outside the four" is at `:2962`.
- **N-2 (1 × 16 floor statistics): partly resolved.**
  - The numbers are right: 20/364 = 5.49%, 924/38,760 = 2.38%, and the stated formula reduces to C(m, r)/C(8 + m, r).
  - The rule allows as few as six valid rounds. At m = 6 the rate is 5.49%, no better than the old median of three (3/55 = 5.45%).
  - At m = 8, which is the fourth run's valid count, the rate is 70/1,820 = 3.85%.
  - So D041-135's reasoning, "a median over more rounds halves the false-stop rate" (`specs/041-step-logs/research.md:146`), holds only from about m = 10 up (2.94% at 10).
  - Remainder: **Major**, disposition fix. Require at least seven valid rounds (the worst case is then 3.85%), or state the true range per m. Also:
    - Give the per-release rate. Over three gates the union bound is 16.5% at m = 6, 11.5% at m = 8 and 7.1% at m = 12.
    - Say what happens with fewer than six valid rounds (not decided, run all twelve again; validity does not depend on the gates).
    - Define the median for an even count. With the mean of the two middle values, 5.49% is an upper bound, not "computed exactly" (`docs/designs/041-step-logs.md:427-429`).
- **N-3 (gate (4) at an unmeasured setting): resolved.** See SC-041-007 at `docs/designs/041-step-logs.md:409-411`, T041-9 at `:3289-3291`, T041-37 at `:3056-3057` and data-model.md at `:472-473`.
- **N-4 (top-up near the limit): resolved in substance.**
  - The normative text agrees: `docs/designs/041-step-logs.md:1419-1425`, ADR-0097 3b at `:132-136`, data-model.md §3b at `:303-318`, and T041-5's tests at `:2525-2530`.
  - Two passages still contradict it (Nit 1 below).
- **N-5 (two-tenant criterion): resolved.** T041-37 now counts only slot holders through `pg_locks` (`docs/designs/041-step-logs.md:3049-3051`) and adds the three-tenant regime (`:3052-3055`).
- **Nits 1 to 6 of my second re-read:**
  - Nit 1 (the cap default) is closed (data-model.md:466).
  - Nit 2 (held-lock tests replace the budget function) is closed (design `:2520`, `:2913`, `:2927`).
  - `resolution.md:248` names correctness Nits 1 to 3 only. Nits 4 to 6 have no answer recorded there, and my second-round report is not in the review folder, so I cannot check them.

### New findings

**B-1 (Blocker, confirmed from the recorded values; disposition: fix, an owner question before T041-9).**
- **Where:** D041-135's new rule, "each gate on the median of all its valid rounds", is stated for both sizes: `docs/designs/041-step-logs.md:413-415`, T041-9 at `:3285-3286` and `research.md:146`. At 8 × 128, gate (2b) is still "at most 0" (`:416`).
- **The data:** the eight valid rounds at 8 × 128 (`specs/041-step-logs/validation-report.md:775`) are −312.64, −25.40, −17.03, −2.19, +6.30, +11.50, +24.68, +112.86.
- **The problem:**
  - Their median is (−2.19 + 6.30)/2 = **+2.06 ms > 0**. The build the fourth run measured fails the gate under the rule the second round introduced.
  - Four of the eight valid rounds are above 0, so an unchanged build fails it with roughly even odds. Under "never re-run", that stops the release.
  - The statement "at 8 × 128 every gate passes" (`:437`) holds only for rounds 1 to 3. The other 8 × 128 medians over the eight rounds pass: (1) 91.8%, (3) 91.9%, (4) 5.
- **Why it was missed:** no one computed the 8 × 128 medians from the newly added table.
- **Fix:** put the 8 × 128 (2b) gate to the owner with these figures. Either derive a floor from the spread, as at 1 × 16, or keep the old statistic at 8 × 128 explicitly and state its false-stop rate (also close to 50%).

**M-1 (Major, confirmed in code; disposition: fix). The connection hand-off cannot be built as written.**
- **Where:** `docs/designs/041-step-logs.md:1416-1418` and T041-36 at `:2815-2817` say the connection is marked `close_on_drop()` at `BEGIN` and that `commit` "clears" the mark when `COMMIT` succeeds.
- **The problem:** in sqlx-core 0.9.0 (pinned at `Cargo.lock:6652-6653`), `src/pool/connection.rs:97-99` only sets the flag to true; nothing clears it.
  - A literal build closes every report connection, so every report reconnects.
  - A build that skips the mark loses the protection.
  - The warm budget test (`:2876-2878`, the backend idle after `COMMIT`) would catch the first build. The design still prescribes a mechanism that does not exist.
- **Fix:** wrap `PoolConnection` in a guard with an `in_tx` state, set before `BEGIN` is sent and cleared after `COMMIT`/`ROLLBACK` succeed. Its `Drop` calls `close_on_drop()` only while `in_tx` is set.

**M-2 (Major, confirmed in text; disposition: fix). The top-up's `limit` answer is unreachable and `held` is undefined.**
- **Where:** `specs/041-step-logs/data-model.md:311-314` answers "`room` when the share is not above the writer's current room, `limit` when it is 0".
- **The problem:**
  - A share of 0 is never above a room of 0 or more, so `room` always wins and `limit` cannot occur.
  - No text gives a condition for `held`: the top-up grants at most F/2 and reclaims nothing (`docs/designs/041-step-logs.md:1421-1423`, ADR-0097 `:134-136`).
  - At the limit the top-up is therefore counted `room`, a skipped case counted as success. That is exactly what T041-37's near-limit regime reads ("top-ups by answer").
  - T041-5 (`:2525`) and T041-36 (`:2922-2923`) require tests that produce `limit` and `held`, which cannot be written.
- **Fix:** test `limit` (share 0) before `room`. Either define `held` (the shortfall is in leases held by open transactions, read without a lock, as the refill does) or remove it from the top-up's answers, the label set and the tests.

**M-3 (Major, plausible; disposition: fix by stating and measuring, or an owner row). The replica cap couples tenants through loss.**
- **Scenario** (`docs/designs/041-step-logs.md:1360-1370`, data-model.md:263-265):
  - With Q = 5 and P = 3, two tenants in the fallback regime (3 + 2 slots) fill the replica cap.
  - A third tenant C, far from its limit, has a carry that falls back (2% of 64-record carries) at a moment when C's own output commit holds C's usage row.
  - C's carry finds no replica slot and answers `service_busy` at once. Under D041-125 alone it would have waited up to 200 ms and written.
- **Gaps:**
  - D041-134 and the ADR consequence (`ADR-0097:321-323`) do not state this channel.
  - T041-37's three-tenant regime gives B no workload that needs a slot.
  - The counted-regime rule (`:3064-3067`) does not exclude A1 to A3, whose busy answers are induced, so it fires by construction.
- **Fix:** state the channel in section 2b and in the risk row at `:2057`. In the three-tenant regime, give B 64-record carries with output and require B's `service_busy` ≤ 0.1%. Exclude A1 to A3 from the counted rule explicitly.

**M-4 (Major, plausible; disposition: fix, qualify the claim and measure it). "At least 5 of 20 never held" covers less than it says.**
- **Where:** `docs/designs/041-step-logs.md:1371-1373`, the risk row at `:2055`, data-model.md:470-471 and ADR-0097:156-157.
- **Scenario 1 (fan-out):** one sibling's fallback holds the run row for up to 200 ms or more. The run's other reports on that replica (16 workers in workload (c)) each hold a pool connection while queued on that row, so tenant B's heartbeats can find the pool empty. The fan-out regime (`:3061-3063`) has no victim tenant.
- **Scenario 2 (transaction pooler):** T041-37 sizes `default_pool_size = 40` for 8 replicas (`:3010`). 8 × Q = 40, so slot holders alone can hold every server backend, before counting 8 × 10 Logs calls.
  - The pool passes its settings through the startup `options` parameter (`crates/infra-postgres/src/pool.rs:69-71`). Through PgBouncer in transaction mode, `sagat.step_log_wait_slots` is per server backend, or the parameter is refused or ignored. Either way Q falls to 1 or varies.
  - The criterion "no heartbeat failing to get a connection" sees only sqlx's pool, not PgBouncer's queue.
- **Fix:**
  - State the bound for direct or session-pooled connections, excluding reports queued on a run row.
  - Add a victim tenant B to the fan-out regime.
  - In the pooler rows, record `SHOW POOLS` `cl_waiting`/`maxwait` for B and `current_setting('sagat.step_log_wait_slots')` on server backends.
  - Check first how PgBouncer 1.21 treats the `options` startup parameter.

### Nits

1. Stale top-up text (N-4 remainder): `docs/designs/041-step-logs.md:1259` says the top-up writes "one write of each quota lease it reclaims" near the limit, and `data-model.md:247` says "the top-up [calls] the refill". Fix.
2. "Reports it to the owner when it exceeds +36.9%" (`:432-433`, `:3309`) does not say which p99 statistic: per round, or the median of valid rounds. Fix.
3. T041-36's "a heartbeat and a task lease of B get their connections" (`:2935-2936`) cannot fail: with 5 or 6 waiters in a large test pool, none of the three mutations exhausts it. Run it at a pool of 8 (Q = 2, P = 1) with the Logs class filled, or drop the claim. Fix.
4. "Tries the usage row `NOWAIT`" (`data-model.md:258`, `:287`): continuing in plpgsql after a NOWAIT failure needs a nested exception block. That is a second subtransaction per fallback, and one in a batch's trigger where none is planned, which contradicts "the refill … open[s] none" (ADR-0097:324-325). Specify `FOR UPDATE SKIP LOCKED` plus an unlocked existence check. The same applies to the top-up's block (`data-model.md:309-310`). Fix.
5. At a pool of 8, tickets equal the cap (4 = 4, `data-model.md:471`), so one tenant can take the whole Logs class. Fix or accept.
6. Correctness Nits 4 to 6 of my second re-read have no line in `resolution.md` (`:248` lists 1 to 3). Fix.

Counts: Blocker 1, Major 5, Nit 6