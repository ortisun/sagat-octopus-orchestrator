# Correctness re-review: task T022-078 (the rate gate's reads, after the fixes)

Reviewer: `correctness-reviewer` (read-only, diff and spec only), 2026-11-20, over the uncommitted fixes on top of
5b2e6de4 on `feature/022-throughput-control`. Text as returned; resolutions in `rate-reads-responses.md`. The diff handed
to the reviewers was taken while a hand mutation was applied to the working tree (see the responses, row one).

I found one blocker: the place read no longer skips waiters past their deadline. There are also two majors, both tests that cannot fail under the mutation they claim, plus four minors and three nits. Nothing was run. Each "confirm with" line names the command to run.

## Blocker

**1. The waiters ahead no longer leave out the ones past their deadline.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/flow_control.rs:1003-1009` (diff lines 418-419).
- What changed: the diff turns `AND x.deadline_at > me.t AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused` into `AND r.status IN (...) AND NOT r.paused`. The deadline filter on the waiters ahead is gone.
- Why it is wrong: the grant pass (`maintenance.rs:225`) still skips any waiter with `deadline_at > now()` false. The doc comment of `RATE_PLACE_SQL` (line 990) and both contracts' `position` text still say "not past their deadline".
- Failing scenario: `limit: 1, per: hour`, a holder, then waiter C (1 s queue) and waiter D. Once C's deadline has passed and before a timeout pass runs:
  - D is shown at position 2 instead of 1.
  - D gets no estimate (k = 1 + 1 - 1 + 1 = 2 > n = 1) instead of the holder's grant plus an hour.
- Likely cause: the comment "a deadline at or before t needs no test of its own" is true for the reading run's own deadline inside `estimated_grant`. It was applied to the waiters ahead, where it is false.
- Tests: the new `rate_rest::the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate` (`rate_rest.rs:582`, assertion at line 623) catches this. So on this tree it fails, and any green run reported for it is not from this tree.
- Fix: put back `AND x.deadline_at > me.t` in `ahead`.
- Confidence: confirmed in code.
- Confirm with: `cargo test -p sagat-tests --test integration_flow_control_step_policies --locked -- rate_rest::the_waiters_the_pass_skips`

## Major

**1. The EXPLAIN test cannot catch the non-index `nextSlotAt` filter it claims to pin.**
- Where: `rate_rest.rs:785`, the check at line 953 (`read <= 1` over `"Actual Rows"`).
- D022-068 item 3 and the new principles row say this test pins "whatever the purge's backlog".
- `Actual Rows` counts the rows a node emits after its filter. It is also averaged over loops.
- Surviving mutation: put back `g.leased_at + make_interval(...) > now()` and `min(g.leased_at + p)` inside the lateral. The grant scan then reads all 601 rows of key `77` but emits 1 (`Rows Removed by Filter: 600`), so `read <= 1` still passes. The `nextSlotAt` value is the same, so that assertion passes too.
- Fix (either):
  - for `rate_limit_grants` and `rate_limit_waiters` nodes, assert that `"Rows Removed by Filter"` is absent or 0;
  - assert that the grant node's `"Index Cond"` contains `leased_at >`.
  - Better still, compare `Actual Rows × Actual Loops` against a total bound, so the per-loop average cannot hide one heavy loop.
- Confidence: confirmed (PostgreSQL 16 EXPLAIN semantics; the mutation was not run).

**2. The STEP-window check of `nextSlotAt` passes without the rule it names.**
- Where: `rate_rest.rs:349`.
- The key has one unleased grant and no leased grant. Drop the `NOT EXISTS (... leased_at IS NULL)` branch and the lateral finds no leased grant, so `nextSlotAt` is still null.
- Fix: give the key a counting leased grant as well, so that only the unleased rule makes it null. Insert a leased STEP grant row, or use `limit: 2` with one step leased by a poll.
- Confidence: confirmed in code.

## Minor

**1. The 256 waiters `nextSlotAt` reads are not the 256 the pass visits.**
- Where: `flow_control.rs:948-953`.
- The pass takes the first 256 eligible waiters: deadline not passed, run running or waiting, not paused (`maintenance.rs:222-227`). The listing takes the first 256 rows, whatever they are.
- Scenario: one key with 256 older waiters of a paused bulk (period hour), then an eligible waiter with period second. The pass serves the per-second waiter. `nextSlotAt` only sees the hour, so it can be later than the real next slot.
- D022-068's reason ("a waiter beyond the first 256 waits behind them anyway") does not hold in that case.
- The contract text ("first 256 waiters in the queue's order") is accurate. The equivalence with the earlier definition, restricted to those 256, is exact (see the sound list below).
- Fix: correct the reason in D022-068 and the plan. Adding the eligibility join is not the fix, because it would bring back an unbounded scan.
- Confidence: confirmed in code; the effect is plausible.

**2. No test reads a key whose waiters have different periods.**
- Mutation that survives: `p.period_ms` replaced by `w.max_period_ms` in the lateral. Every test uses a single period.
- Fix: one key with an hour waiter and a minute waiter, and a lease 30 minutes old. Expected `nextSlotAt` is that lease plus the hour; under the mutation it comes back null.
- Confidence: confirmed.

**3. Every failure of the place read is swallowed, corruption included.**
- Where: `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/application/src/runs.rs:2395-2410`.
- `corrupt("rate-limit period")` and the other `corrupt` errors now become a warning and a missing place, where before they were a 500.
- The contract says null "when the place could not be read in time".
- The log carries the whole error's text, where D022-068 item 2 says the error's kind.
- Fix: swallow only a statement timeout (57014), a pool timeout and a connection error. Let `StoreError::Corrupt` propagate. Log the kind only.
- Confidence: confirmed in code; the impact is judgement.

**4. `t` is now the transaction's start, not the statement's.**
- Where: `flow_control.rs:818-845`.
- `now()` is the BEGIN instant, three round trips before the snapshot. A run whose deadline falls between BEGIN and the statement can still be shown "room now" at `t`.
- The impact is tiny. Using `statement_timestamp()` for `t` in `me` (or `clock_timestamp()`) restores the old meaning.
- Confidence: confirmed; nit-level impact.

## Nit

1. `rate-reads-responses.md` is not in the diff and still says "Open: next change" for every finding fixed here.
2. `one_groups_rate_keys_page_through_the_keys_nobody_waits_for` only checks that the pages equal the unpaged list. The tie order "pool, scope, digest" in its doc comment is never checked against an independent expectation.
3. `PlaceReading` is `pub`, but the "no separate deadline test" argument depends on `kth_lease > t - per`, which only the SQL guarantees. Note that on the struct, or add a `debug_assert!`.

## Checked and sound

- **Edges of k:** `estimated_grant` handles k ≤ 0 (gives `t`), 1 ≤ k ≤ n (the k-th lease plus `per`) and k > n (`None`), with `saturating_add`. The unit tests hit k = -2, 0, 1, n, n + 1, n = 0, a missing lease and a saturated sum. Each of these mutations fails a unit test: `<` to `<=` on the room test, `>` to `>=` on `k > counted`, and `<` to `<=` on the deadline.
- **Deadline rule:** `at < deadline` matches the pass's `deadline_at > now()`. Equality gives `None`, and 1 µs either side is tested for both room and lease instants. "A deadline at or before t needs no separate test" holds for the run's own deadline: `at` is `t`, or a lease strictly after `t - per` plus `per`, so `at` is never before `t`.
- **The rewrite of `nextSlotAt`:** `min(leased_at) + p` over `leased_at > now() - p` equals `min(lease + p)` over `lease + p > now()`. Periods are 1, 60 or 3600 s, so the interval has no day or month part and the arithmetic is exact in microseconds. Adding `p` is monotone, NULLs are ignored the same way, and an empty set gives null in both. `DISTINCT` after `LIMIT 256` in the ordered subquery means the periods of the first 256. The `const` assertion ties the literal to `GRANTS_PER_KEY_PER_PASS` (u32 256). The order matches `rate_limit_waiters_order_idx`.
- **Own transaction of the place read:** `begin_safely`, then `READ ONLY` (allowed after `begin_safely`), then `SET LOCAL statement_timeout`, which also covers lock waits during Parse and Bind. On error the transaction is rolled back on drop.
- **The timeout test:** `a_place_that_cannot_be_read_in_time_is_left_out_and_the_run_is_shown` follows the real order: the run row, then `reveal_permit_labels`, then `rate_place`. It uses held locks and `pg_stat_activity`, no sleeps. It cannot pass by accident: without the timeout the join hangs until the 60 s bound, and with a propagated error the read returns 500.
- **`each_waiter_is_estimated_from_its_own_counted_lease`:** catches `OFFSET 0` and `DESC`, with g1 < g2 asserted.
- **`an_estimate_after_the_queue_deadline_is_withheld`:** only the deadline rule can make the estimate null there.
- **Queue timeout in the fixtures:** the two-hour queue in `rated_start` is below the 24 h ceiling and the 10-minute default, so estimates are not hidden by a cap.
- **`rate_limit_windows.oldest_wait_at`:**
  - the consistency row compares `IS DISTINCT FROM` per window of the tenant;
  - `must(20_270_207_000_000, …)` makes a missing column a failure, never a pass;
  - the tamper test is 1 µs off;
  - the delete-one, delete-all and replace-older or replace-younger tests would each fail if the trigger left the value unchanged.
- **The `rate_gate.rs` addition:** it checks the remaining waiter's instant exactly.
- **`check-architecture.py`:** the third file is covered, and the tests now expect two missing-file errors.
