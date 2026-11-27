# Correctness review of task T022-080 (rate fault suite) and decision D022-071, 2026-11-21

Reviewer: `correctness-reviewer` (read-only), given the uncommitted diff over `ab9d85a4`
(`crates/infra-postgres/src/execution/gate_queue.rs`, `tests/fault/flow-control-step-policies/{main.rs,rate_limits.rs}`)
and the specification. Saved verbatim. Not yet answered: the task stays unticked.

---

The change itself is sound for the case it targets. I found no Blockers. The Majors are mostly tests that cannot fail on the defect they name, plus one projection the change leaves behind. I read the code and ran nothing, so every verdict below comes from reading the code.

**What holds up**
- **New statement in the grant transaction.** `rate_ceilings` (`gate_queue.rs:99-109`) is a plain read of `quota_limits`/`quota_ceilings` after the key locks. It takes no row lock, so the lock order is unchanged. Admission already reads the same tables, so the runtime role can read them too.
- **Types.** sqlx declares `$13..$15` as int8, so `least(int4, CASE … int8 … ELSE int4)` resolves to bigint against `taken.n` bigint. `ceiling_of` caps u64 at `i64::MAX`. The `ELSE w.declared_limit` branch is unreachable under the `period_ms IN (…)` CHECK and matches `period_of → None → own`. `LIMIT $10` stays the filed limit, which is at least the effective limit, so it never truncates a count that matters.
- **Agreement with admission.** Admission files `declared_limit = min(declared, ceiling at admission)` (`readiness_gate.rs:265`) and refuses at `taken + waiters >= limit` (`:359`). With a lowered ceiling, deciding against `min(filed, current)` agrees with what a new admission would apply.
- **Single-period agreement.** For a key with one period, the deciding rule and the waiters-ahead rule agree. Half-open windows are consistent: `leased_at > now - p`, and the test checks `(t - p, t]`.
- **Sleeps.** There are none. `until_past` and `barrier` poll with `yield_now` under a 120 s bound.

## Majors

**M1. Rounds 0 and 1 of grant-versus-timeout do not race the grant decision.** `rate_limits.rs:607-632` (10 of the 20 rounds). Confirmed in code.
- **Scenario:** `age_start_deadline` makes the start due before the grant pass. `gate_key_waiters` filters `w.deadline_at > now()` (`maintenance.rs:225`), so the pass lists nothing. It then calls `rest_gate_key`, whose `SELECT … FOR UPDATE` on the window (`maintenance.rs:281-295`) blocks on the held window.
- **So `barrier(&w, 1)` is met by the rest statement, not by `grant_from_queue`.** The grant never sees the start. Round 1's "timeout first" is the same situation.
- **Why tests miss it:** deleting the `deadline_at <= now` check in `grant_from_queue` (`gate_queue.rs:277`) still passes rounds 0 and 1. Only round 3 (5 rounds, one order) exercises it. "Timeout commits first while the grant has the start in hand" is never tested.
- **Fix:** in every round where the grant should decide, set the deadline a short offset ahead and let the grant list and block before `until_past`, as round 3 does. For timeout-first: hold the run so the grant pass, which listed the start before its deadline, reports it Busy. Assert `g.skipped == 1`, not just `gates_granted == 0`.

**M2. The database-clock set-back test never runs a pass while the moved grant is in the future.** `rate_limits.rs:1185-1215`. Confirmed in code.
- **Scenario:** the holder's grant is moved to `t0+1s`. The test then calls `until_past(t0+1s)` before the first pass, so at every decision the grant is already in the past.
- **The defect the spec edge case names would pass:** "grants counted in the future keep counting". For example, an added `AND g.leased_at <= $12` in `admits`, `key_count` or the rest hint gives identical results in `[t0+1, t0+2)`, and `granted >= t0+2s` still holds.
- **The one direct assertion is also conditional** (`if after < t0 + 2 s`, line 1197) and is silently skipped on a slow runner.
- **Fix:** move by several windows (for example `+5 s`) and run a pass immediately, while the database clock is below the moved instant. Assert `gates_granted == 0` unconditionally, guarded by a clock read before the pass. Then assert the grant is no earlier than `moved + 1 s`.

**M3. The place estimate and the run's gate view still use the filed limit, so they diverge from the grant rule.** `flow_control.rs:1015-1040` (`RATE_PLACE_SQL`, `me.declared_limit` at 1033/1038); plan.md:665/675. Confirmed in code.
- **Scenario:** filed `limit: 100, per: hour`, ceiling lowered to 10, `n=100` counted, `a=50` ahead.
  - The estimate computes `k = 100+50-100+1 = 51` and shows "51st lease + 1 h".
  - The routine (`n + a < 10`) actually needs about 5 more windows. With `L=10`, `k = 141 > n`, so the estimate should be absent.
- **Before this change the two agreed on `L`; the change splits them.** The run view also shows `limit` 3 while decisions use 1 (plan.md:665 says "the effective limit the row was decided with").
- **Why tests miss it:** no `rate_rest.rs` test lowers a ceiling.
- **Fix:** read the namespace ceilings in `rate_place` and use `least(me.declared_limit, ceiling(me.period_ms))` as `L`, in `OFFSET` and in the returned limit. Add a `rate_rest` case with a lowered ceiling.

**M4. The waiters-ahead predicate compares against the wrong window for a key with mixed periods. For single-period keys the new clause changes nothing.** `gate_queue.rs:237-239`. Pre-existing; this diff extends the predicate. Confirmed in code, untested.
- **Scenario:** key K is shared by A `{1/hour}` and B `{1/second}`, which the spec allows with a warning (spec.md:324-326).
  - A's holder is leased at t0. A's waiter Wa queues at t1, then B's waiter Wb.
  - Deciding Wb counts `taken` over one second, giving 0.
  - Wa is counted ahead because `least(1, C_hour) > 0`, so Wb is refused for up to an hour.
  - Wa is itself refused by its own limit (one grant in the hour), so this is the head-of-line blocking the spec forbids.
- **The new `least(…, ceiling of its own period)` is still compared with `taken` of the deciding waiter's period.**
- **For one period the clause is a no-op:** if `min(L_a, C) <= taken < L_a`, then `C <= taken`, so the deciding waiter is refused anyway. Reverting the ahead clause fails no test.
- **Fix:** count `taken` per ahead waiter over its own `period_ms` in a correlated subquery, or document the restriction against D022-009. Add a mixed-`per` grant-pass test.

**M5. The flood test's admission-order assertion depends on runner speed, and on a slow runner it exposes a real deviation from SC-022-017.** `rate_limits.rs:906-916, 980-983`. Plausible.
- **Scenario:** admission grants whenever `taken + waiters < limit` (`readiness_gate.rs:359`).
  - Starts 1-10 are granted. Starts 11-14 queue.
  - If more than 1 s has passed before the buffer holds 10 waiters, start 15 sees `0 + 4 < 10` and is granted at admission.
  - The pass grants 11-14 later, so `in_order` fails.
- **On a fast machine the loop finishes in under 1 s and the deviation is hidden.** On a loaded shared machine (roughly 50 ms per start) it fails.
- **Fix:** decide which is intended. Either admission queues whenever the key has an eligible waiter (strict FIFO), or the reservation rule stands and SC-022-017's wording changes. Then make the test deterministic: age the first ten grants explicitly before start 15 and assert the chosen behaviour.

**M6. In T20, T21 and T24 the process dies before the decision writes anything, so "nothing half done" is never tested.** `rate_limits.rs:328, 358, 527`. Confirmed in code.
- **What is held:** every crash kills the backend waiting for the held window lock. `grant_from_queue`, the timeout and the cancellation take the window lock before any write. The waiter delete, grant insert and window hint come after it, and `engine.run`'s journal and outbox come after `grant_queued_step` (`passes.rs:5780-5819`).
- **Why tests miss it:** a refactor that committed the grant rows and then opened a new transaction for the journal would pass these tests. SC-022-018 asks for a crash at every commit point.
- **Fix:** add a second crash point per path that blocks the last statement. For example, hold an uncommitted conflicting `run_events` row (the run's next `seq`) or a conflicting `event_outbox` key, so the deciding transaction waits on the unique index after its gate writes. Kill it there and assert grant rows, waiter rows and journal are all unchanged.

## minors

- **m1. Cancel race: "the run cancelled either way" is not asserted.** `rate_limits.rs:735-745`. Only the event counts and the waiter row are checked; `cancelled.expect` checks `Ok`. Add `assert!(w.run_status(waiter).await.is_terminal())`, and for even rounds check the slot row the spec expects. Confirmed.
- **m2. T23's "never grants early" cannot fail.** `rate_limits.rs:512-516`. With `per: hour`, the waiter cannot be granted for an hour whatever the hint is. The hint assertion only shows the killed statement did not commit. Use `per: second` and assert the waiter's grant is no earlier than `leased_at + 1 s`. Confirmed.
- **m3. Spec drift on a raised ceiling.**
  - The spec (spec.md:321-323) says the "pinned `limit` … clamped by the current policy".
  - The filed limit is already clamped at admission (`readiness_gate.rs:265`), so a ceiling raised later never lifts a waiter back to its declaration (under-delivery).
  - D022-071 item 3 says "never lifts a waiter above its declaration", but the bound is actually the admission-time ceiling.
  - `RateGranted.limit` and the grant row's `declared_limit` (`gate_queue.rs:342, 363`) record the filed 3, not the 1 the grant was decided with.
  - Fix: either store the declaration, or correct D022-071 and the spec sentence, and journal the effective limit. Confirmed.
- **m4. Two documents still name `libfaketime`.** D022-072 amends the method, but plan.md:753 and the tasks.md:450 amendment still say `libfaketime`. Update both in the same delivery (rule 10). Confirmed.
- **m5. SC-022-017 is only partly covered.** Its second clause (a second key with a saturated buffer, and more keys or tenants than one pass visits) has no test that names it. The flood test covers one key. Plausible; it may be covered under another id.
- **m6. Round 3 can fail falsely on a slow runner.** `rate_limits.rs:592-595`: the 2 s deadline margin must cover `hold`, the spawn and the listing. It fails rather than passes falsely. A wider offset costs only test time.

## nit

- **n1. One extra statement per waiter tried.** `rate_ceilings` runs once per waiter tried (up to 256 per key per pass). Reading once per key per pass gives the same freshness; per-statement reads under READ COMMITTED are not serialized against a quota change anyway.

Commands that would turn the plausible findings into evidence:
- M1: delete `gate_queue.rs:277-279`, then run `cargo test -p sagat-tests --test fault_flow_control_step_policies rate_limits::a_grant_and_a_timeout -- --nocapture`. Expect rounds 0 and 1 to pass.
- M2: add `AND g.leased_at <= $12` to the rate `taken` CTE and rerun `rate_limits::a_database_clock_set_back`.
- M5: run the flood test under `RUST_TEST_THREADS=2` with a parallel heavy suite.

Blockers: 0, Majors: 6
