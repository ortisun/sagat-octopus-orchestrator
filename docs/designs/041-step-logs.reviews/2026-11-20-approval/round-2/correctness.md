# Correctness re-read of design 041's fixes (approval run, second round)

Reviewed at `4d6fe4cc`, against my first-round report at `ff8033cd`. All fifteen first-round findings are resolved by the text. The fixes bring in five new Majors and no Blocker.

I checked every number the fixes added and each one is arithmetically right:
- **Gate (1):** median 0.8199, worst 0.8046, tolerance written 1.6, floor 80.4.
- **Gate (2b):** median 8.6247, worst 29.0307, tolerance written 20.5, floor 29.1.
- **Gate (3):** median 0.8709, worst 0.8352, tolerance written 3.6, floor 83.5.
- **Gate (4):** worst valid round 10, in rounds 4 and 6.
- **Ticket defaults:** 6.44 → 7, 4.22 → 5; 43% headroom (1/0.7 − 1).
- **Top-up figures:** 24,704, 983,040, 299.7, 0.33% (0.325%); 1,000 ms = 800 + 200.
- **Pooler:** 160 / 40 = 4.
- **Re-run:** at most four transactions. T041-36 lines 2857-2858 confirm that a failed content fallback is re-run as the fallback.

What is wrong is how some of these numbers are used. That is the subject of N-2, N-3 and N-5.

## First-round findings

| Id | Status | Where |
|---|---|---|
| C-1 | Resolved. The criterion is written per size, the tolerance is stated, and the Goals and T041-9 cite it. The floor's own problems are N-2 and N-3. | design 397-431, 446-452, 3182-3185 |
| C-2 | Resolved | design 371-374 |
| C-3 | Resolved. No other transaction's answer feeds the plan, and the property test covers each answer of the block. | design 1142-1150; ADR-0095 79-81 |
| C-4 | Resolved for (a) (an explicit `path`, `room` null only with `none`), (c) (one producer per metric, sequences count attempts) and `mark_only_fallback`. The leftover of (b) is counted once, as N-1. | data-model 478, 484-487, 496-499, 600-606 |
| C-5 | Resolved. The kept room is gone, and the top-up has boundary tests at 65,535 and 65,536 and a same-backend test. | design 2838-2846, 2470-2473 |
| C-6 | Resolved. ADR-0095 decision 4 holds again; its Note covers the fallback wait after the fence checks. | ADR-0095 84-94; design 184-194, 1232-1235 |
| C-7 | Resolved | design 983-985 |
| C-8 | Resolved | design 446-449, 1291 |
| C-9 | Resolved (box unticked, reason stated) | design 2298-2307 |
| C-10 | Resolved | design 1265-1278, 2800-2806; data-model 367-368 |
| C-11 | Resolved (moot: the top-up takes no amount) | data-model 297 |
| C-12 | Resolved | design 2502-2505 |
| C-13 | Resolved | design 1243, 1245 |
| C-14 | Resolved | data-model 216-217; design 2428-2430, 2527 |
| C-15 | Resolved | design 1357; data-model 279-280, 320-322 |

## New Majors

**N-1 — `failed` is missing from the task that builds the function, and its test asserts the old re-run.** Confirmed in the text.
- **Where:**
  - Design T041-36, lines 2733-2734: the function refuses anything "other than `invalid`, `secret` and `content_fallback`".
  - Line 2867 says "outside the three".
  - Lines 2855-2858: a guard's raise re-runs "with no carry … no final mark".
  - data-model §1, line 19, gives a three-value subset.
  - data-model line 594-595 lists the `dropped_by_server` keys without `failed`.
- **What the rule says instead:** design 2a 1123 and 1217-1221, data-model §8a items 0 and 590, ADR-0095 decision 5 and D041-128 all say a guard's raise re-runs with a mark-only carry whose reason is `failed`, and the stream keeps its final mark.
- **Scenario:**
  1. Transaction 1 (Full): a quota-lease `CHECK` raises, mapped to `StepLogRefused`.
  2. Transaction 2 (MarkOnly with reason `failed`): a function built to T041-36 raises at step 0, mapped to `StepLogCarry`.
  3. Transaction 3 (None): no final mark is written.
- **Why no test catches it:** T041-36's test asserts exactly that outcome. The specified behaviour has no test, and a correct build fails it.
- **Fix:** allow four reasons in all three places. The guard-raise test should assert two transactions, a mark-only final mark and `failed` counted. The failed-call test keeps None.
- **Disposition:** fix.

**N-2 — the 1 × 16 floor can stop the release on an unchanged build.** Plausible.
- **Where:** design 413-422 and 3184-3185; D041-126 ("cannot fail it by construction").
- **Why:** each floor is the worst of eight valid rounds, but T041-9 judges on the median of three new rounds. Under exchangeable rounds, an identical build fails a gate when the two lowest of 11 values are both new: 3/55 ≈ 5.5% per gate. Over the three gates that is roughly 10-16%.
- **Margins:** 0.06 points for gate (1), 0.07 ms for (2b) and 0.02 points for (3).
- **The void rounds already went past it:** gate (3) at 82.15% and 83.23%, gate (2b) at +33.3 ms.
- **The text is wrong:** "fails only for a build slower than the spread" does not hold.
- **Fix:** judge 1 × 16 on the median of all valid rounds of a four-runner run (at least six), or set the floor beyond the worst round. Correct the sentence either way.
- **Disposition:** fix, or accept with the false-stop rate stated.

**N-3 — gate (4) will be judged at a ticket default nobody measured.** Confirmed in the text; the effect is plausible.
- **Where:** T041-9 3169-3170 runs "with the ticket semaphore set to T041-7's defaults" (7). The fourth run used "cap and tickets 10" (data-model 453; design 1045).
- **Why it matters:** two of the eight valid rounds needed 10 tickets (rounds 4 and 6). Refusals under 1% were measured with 10 tickets, never with 7. D041-126's claim that the measured build meets the floor does not cover gate (4)'s refusal clause.
- **Fix:** measure workload (g) at 7 tickets (in T041-37) before T041-9. Or judge gate (4)'s formula alone and record the refusals at 7 with a stated consequence.
- **Disposition:** fix.

**N-4 — near the limit, the top-up after every carry takes other backends' room and wait slots.** Plausible.
- **Where:**
  - data-model 283-285 and 299-304.
  - ADR-0097 decision 3b 129-132 and decision 4 169-174.
  - design 1386-1388 and 1471-1476.
- **Mechanism:**
  1. With share < M, every carry returns `top_up` true.
  2. Each top-up calls the refill with `bytes` 0. That reclaims every idle quota lease whenever F < max(0, 1 MiB).
  3. It then folds and regrants the writer's lease. The new grant can be below the room it had, for example 60,000 → F/(2(A+1)) = 7,500 when A = 3 and the rest is free.
  4. It waits under the wait slots.
- **Effect:** a speculative statement, off the request path, empties other backends' quota leases. Their next carries fall back under the run lock, and they find the slots taken by top-ups, so they answer `service_busy`.
- **Why the tests miss it:** T041-37's near-the-limit regime records fallbacks but judges only U ≤ L, busy ≤ 0.1% and the longest carry.
- **Fix:** the top-up should try the usage row `NOWAIT` only (no slot, no wait, `busy` at once), never reclaim, and grant only when the share is above the current room (otherwise answer `room`).
- **Disposition:** fix.

**N-5 — the two-tenant criterion counts waits the slots never bound.** Plausible.
- **Where:** T041-37 2943-2948. The claim it tests is in design 1349-1350 and D041-125 ("all of one tenant's waits on its usage row").
- **Why it can fail by construction:**
  - Workload (a) returns a 1 KiB output.
  - A completion with output takes the usage row at its commit, for the content class, with no wait slot (design 1025-1028).
  - Tenant A has 64 of 128 workers, which is 8 per replica.
  - With A's row held 150 ms of every 300 ms, 8 waiting backends per replica is reachable.
  - So "no more than seven waiting on A's usage row" can fail with no defect, and the stated consequence is a design change.
- **Fix:** count only backends that hold a wait slot (`pg_locks`, the two-int4 advisory key), or run tenant A on workload (f). Scope the claim to step-log waits, as data-model 263 does.
- **Disposition:** fix.

## Nits

1. **Stale cap default.** Design 835-836 still says the Logs cap defaults to "a quarter … 5 of the … pool of 20". Lines 848-850 of the same paragraph, data-model 452-454 and D041-127 say half (10 of 20).
2. **Held-lock tests depend on timing.** These tests need every waiter to arrive within *b* = 200 ms:
   - T041-5 2466-2469: eight top-ups, seven waiting.
   - T041-36 2847-2851: nine reports, "exactly seven" waiting and two busy.
   - T041-36 2841-2842: the answer arrives "while its top-up still waits".

   Replace the budget function with the test's own bound, as the limit test does at 2834-2835 (rule 18).
3. **The slot key reads a session setting.** The key reads `current_setting('application_name')` (data-model 256). But data-model 261-262 says these functions never read a setting a caller can change, and T041-5 2500 tests that "no session setting is read". Also, "stricter, never looser" (data-model 265-266; design 1353) is false if names are more distinct than replicas. State the exception and that only the server sets the name.
4. **`path = refill` is mislabelled in a race.** It is defined as "its block ran and its fallback refill granted" (data-model 600-601). But the block's charge can succeed with no refill: the reserve reads S + G > L without a lock, then the charge finds room.
5. **The cold start may fall outside the measured window.** Rows are measured after a 30 s warm-up (design 2907-2908). Unless stated, the cold-start regime's busy carries land in the warm-up and its 0.1% criterion cannot see them.
6. **Counters can count a rolled-back carry.** The store counts from the carry's return (data-model 478-483). Scenario: a report whose 1 KiB output exceeds `tenant_content_bytes`, with a carry that fell back. Its commit raises, the content fallback commits a `content_fallback` mark, and `fallbacks_total` still counts the rolled-back refill. Count only the committed transaction's carry.

Counts: Blocker 0, Major 5, Nit 6