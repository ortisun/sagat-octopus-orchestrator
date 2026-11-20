# Correctness review: feature 022 rate gate (T022-072, T022-074, T022-077, verify_run invariants 8-10 for the run start)

Reviewer: `correctness-reviewer` (read-only, diff and spec only), 2026-11-20, over the uncommitted change later committed
on `feature/022-throughput-control`. Text as returned; resolutions in `rate-gate-responses.md`.

Worktree: `/Users/marcelo/Documents/Projects/sagat-wt-022`, branch `feature/022-throughput-control`, uncommitted diff. I could not run anything. Each finding names the command that would confirm it.

**Summary:** 5 Major, 6 Minor, 4 Nit. Two of the Majors leave a waiter stuck until its queue deadline although a slot is free. One lets the gate grant more than `limit` in a window, against the journal's own instants. One breaks the architecture-checker test suite.

## Major

### M1. The checker's test file was garbled; a test for a removed rule is left behind and fails (confirmed in code)
`scripts/tests/test_check_architecture.py:966-1329`

- The hunk `@@ -962,6 +962,352 @@` does not delete `test_the_rate_gates_feature_of_the_postgres_adapter_belongs_in_dev_dependencies_only`. Instead it inserts a second copy of 346 existing lines: a mid-file `if __name__ == "__main__": unittest.main()`; `run_input_check`, `TheRunInputValidatorIsThePlatformsOwn`, `TheApplicationHoldsNoRuntime`, `creator_check`, `creator_tree`, and the whole `EveryRunIsCreatedWithACheckedInput` class.
- The stale rate-gates test (lines 1311-1325) now sits in the second `EveryRunIsCreatedWithACheckedInput`, which shadows the first.
- The rule it checks was removed, so `assertEqual(code, 1)` fails.
- Every repository-wide `subprocess.run([... SCRIPT])` test in the duplicated block runs twice. None of the Rust suites run this file.
- Confirm: `python3 -m unittest scripts/tests/test_check_architecture.py -k rate_gates`.
- Fix: restore the file from the base, then delete only that one test method.

### M2. A run's slot is granted at an instant earlier than the one its window is counted at, so invariant 10 can be breached (confirmed in code; timing-dependent)
- Where: `readiness_gate.rs:345` reads `now`; `:649-659` counts with `g.leased_at > clock_timestamp() - period`; `gate_queue.rs:138` reads `now`; `:165-169` counts with `clock_timestamp()`.
- The decision instant `now` is read first. The count runs one or more statements later, at `c > now`. The grant is journaled with `granted_at_us = now`, and for a run `leased_at = granted_at`. A grant A with `now < G_A + p <= c` is not counted, yet it lies in B's window `(now - p, now]`.
- Scenario (`rateLimit: {limit: 1, per: second}` on a run start): grant A at `G_A`; start B reads `now = G_A + 1s - 50µs`; B's count runs at `G_A + 1s + 20µs`, so A is not counted and B is granted; the journal holds two grants inside one 1-second window; `verify_start_rate` reports "invariant 10 … fails".
- Admission: the chance is about RTT / period per window boundary, non-negligible at the SC-022-029 load. Grant routine: only when the pass was made due by something other than the age-out hint.
- The tests always cross the boundary by a wide margin, so `second_at >= first_at + 1s` (`rate_gate.rs:183-186`) holds whatever the code does.
- Fix: bind the decision instant into both counting statements (`g.leased_at > $now - make_interval(...)`), and into the `deadline_at > …` predicates for consistency.

### M3. The rest rule is computed at a later clock than the counts it follows, so a waiter can be left with no hint (plausible; race in code)
- Where: `maintenance.rs:256-275`, `rest_gate_key` for rate; filter `g.leased_at + p > clock_timestamp()`.
- `grant_key` lists the waiters, tries each in its own run transaction with its own count, and only then rests the key. A grant that ages out between the tries and the rest was counted as taken (the waiter refused), then dropped by the rest, which picks a later expiry or `NULL`.
- Scenario: `limit: 1, per: hour`; grant A leased at L; W arrives at `L + 1h - 2ms`; the pass counts at `L + 1h - 1ms` (W refused); the rest at `L + 1h + 1ms` sets `NULL`; W times out with a free slot all along.
- Fix: rest from the pass's start instant `t0` rather than `clock_timestamp()`; a grant that aged out during the pass then yields a hint `<= now` and the key is due at once.

### M4. A grant made through another key's pass does not move the window's hint, so a waiter behind the reserved head is stuck (confirmed in code; plan drift)
- Where: `gate_queue.rs:201-307` (`grant_from_queue`), the rest at `maintenance.rs:256-275`, plan "The grant pass, by key" (`plan.md:546-548`: "a grant that leaves the key full sets it to the instant the oldest counted grant ages out, or to now plus the shortest period when an unleased grant holds a slot").
- No grant sets a window's hint; only `key_due` (a new waiter) and `key_is_eligible` (a return) do.
- Scenario (two definitions share rate group `r`, key `k`, both `limit: 1/second`): W1 dual-gated (rate plus concurrency `c`, held), W2 rate-only behind it; the window pass refuses W1 (its permit) and W2 (the reservation), rests `NULL`; `c` is released, the semaphore's pass grants W1 both gates; nothing wakes `k`; W2 times out.
- Fix: whenever a rate grant is inserted on a window with `waiters > 0`, set `next_eligible_at = least(coalesce(next_eligible_at, 'infinity'), now + shortest waiter period)` under the window's lock already held.
- The pre-diff rest rule had the same `NULL` outcome. D022-064 item 5 calls the new rule "exact … early, never late", and it is not.

### M5. A reset of a rate-limited run is refused with a retryable 503 by every process of this build, and a pending reset is retried forever (plausible)
- `readiness_gate.rs:1088-1095` refuses `StartCause::Reset` with `rateLimit` as `BINARY_CAPABILITY_MISSING`; `reset_completion.rs:241-243` maps it to `None` ("another process completes the entry"); `:380-386` backs off.
- Before this diff no run with a run-level `rateLimit` could exist; now they are admitted.
- Synchronous reset: 503 `UNAVAILABLE`, retryable per `contracts/errors.md:28`; clients retry a refusal no process will lift. Pending reset: never resolves, keeps its `pending_resets` quota.
- No test resets a rate-limited run. Fix: a terminal, non-retryable reason (018's `RATE_LIMIT_SLOT_UNAVAILABLE`, or refuse at reset-request validation), with tests of both paths.

## Minor

- **m1.** A dual-gated start whose concurrency queue is full is refused `RATE_LIMIT_QUEUE_FULL` (`readiness_gate.rs:1496` maps `KeyWaiters` to the rate refusal whenever `declared.rate`); `contracts/errors.md:22` says `CONCURRENCY_QUEUE_FULL`. The unit test builds this behaviour in. Fix: pass the refusing gate's kind into `start_refused`.
- **m2.** The rest hint can be late by the lease pass's lag (`maintenance.rs:259`: `coalesce(g.leased_at, clock_timestamp()) + p`): a lease journaled at `L < now` but not yet copied frees at `L + p`. D022-064's deadlock argument does not hold: a return deletes only `leased_at IS NULL` rows of an attempt whose journal has no lease and the lease pass only updates rows whose journal has one, so they never contend. Fix: after the lease UPDATE, lower the hint to `leased_at + min waiter period` on windows with waiters.
- **m3.** `rate_gate.rs:80`, `:147`, `:162`: `hint_us` runs `fetch_one` with no `ORDER BY` over two matching windows (`acct-1`, `acct-2`); filter by the run's key.
- **m4.** `verify_start_rate` has no negative test (T022-079's evidence asks for one); the "purged" branch is never exercised; invariant 10 from the journals contradicts G022-017 for a later hourly acquirer; invariant 9's `grants_total` is not checked.
- **m5.** `rate_gate.rs:183-186` is vacuous (`until_past` guaranteed it); `rate_gate.rs:529-532` asserts on wall-clock progress and says "rerun on a quieter machine", a timing assertion standing in for an order (rule 18).
- **m6.** T022-072's evidence lists event, cron and child starts, none tested for a rate-limited start; also missing: a delayed rate-limited start's release, a dual-gated start granted from the queue, a dual-gated timeout, the step-level `rate_limit_live_keys` refusal and the per-window clamp at readiness.

## Nit

- `passes.rs:~5587-5595`: the new `rate_leases` doc was inserted after `gate_grants`'s doc lines.
- `maintenance.rs:1255-1265`: the purge's candidate scan uses no index; the per-key `FOR UPDATE` has no `SKIP LOCKED`.
- `maintenance.rs:301-324`: the lease pass probes every unleased grant of each shard every tick before `ORDER BY … LIMIT`.
- `rate_limit_live_keys` usage counts windows of every key version; the gate only versions in force. The runbook's G022-017 text also applies to a longer-window acquirer that arrives first in a window row's life.

## Checked and sound

Both-or-neither for a dual-gated start with one deadline; lock order (semaphores then windows sorted; purge window before grant rows; lease pass grant rows only); the lease pass picks the right attempt (re-grants after a pause handled; `greatest(…, granted_at)` plus the CHECK); recording is idempotent and fence-free safely; purge boundary matches FR-022-056 and `max_period_ms` covers waiters; half-open windows agree across count, rest and verify; microsecond conversions agree; `queued_runs` counts rate waiters and a refused admission writes nothing.
