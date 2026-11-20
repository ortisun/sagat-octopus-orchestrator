# Correctness review: task T022-078 (the rate gate's reads)

Reviewer: `correctness-reviewer` (read-only, diff and spec only), 2026-11-20, over the local commit after d08bb79b on
`feature/022-throughput-control` (diff `d08bb79b..HEAD` without the generated Studio types). Text as returned;
resolutions in `rate-reads-responses.md`.

**Summary**

I found no blocker and no wrong result in the code. The arithmetic matches the plan and the grant routine (`admits`) in every case I traced. The trigger replacement keeps every counter the old function kept, and the down migration restores the old function word for word. The real weakness is the tests. The integration tests in `rate_rest.rs` cover only the simplest cases: one counted grant, live waiters, waiters never removed. Several hand mutations that change what users see would still pass, including the central "k-th lease" step of the estimate. There are three Majors (all test gaps on claims the design states and depends on), six Minors (including two small behaviour gaps and one possible link between runs for readers without the reveal grant) and four Nits. None of the mutations below has been run.

Every confirming command below has the same form. Apply the named hand mutation, then run:
`cargo test -p sagat-tests --test integration_flow_control_step_policies --locked -- rate_rest`

**Blocker**

None.

**Major**

- **Major 1. The k-th lease (k ≥ 2) and the order of the leases are never tested.**
  - Where: `crates/infra-postgres/src/flow_control.rs:983` and `crates/application/src/flow_control_keys.rs:235`.
  - Every test has `n = 1`, so the OFFSET is always 0, or the k > n case returns `None` before the lease is used.
  - Two mutations would survive: `OFFSET 0`, and `ORDER BY g.leased_at DESC`.
  - Scenario that would catch them: `limit: 2, per: hour`, holders granted at g1 < g2, waiters w1 and w2. The expected results are w1 = g1 + 1h (k = 1) and w2 = g2 + 1h (k = 2, OFFSET 1). A third waiter w3 should get null (k = 3 > n).
  - Fix: add that test, plus a unit test of `estimated_grant`, which has none. It should cover k ≤ 0, k = 1, k = n, k = n + 1, and `kth = None`.
  - Confidence: confirmed in code. Confirm with the `OFFSET 0` and `DESC` mutations.

- **Major 2. The filters on the waiters ahead are untested.**
  - Where: `flow_control.rs:977`.
  - D022-067 item 2 chose this rule specifically to skip waiters past their deadline, waiters whose run is paused, and waiters whose run is no longer live. No test has such a waiter ahead.
  - Mutation that survives: delete `AND x.deadline_at > me.t AND r.status IN ('RUNNING','WAITING') AND NOT r.paused`.
  - Scenario: `limit 1/hour`, a holder, waiters A, B and C, then pause A's run. C should be at position 2 (it shows 3 if the filter is gone). The same should hold for B after B's run is cancelled, or with a waiter whose deadline has passed but which has not yet been timed out.
  - Fix: add these three cases.
  - Confidence: confirmed in code.

- **Major 3. Nothing checks how `oldest_wait_at` is kept up to date when a waiter is removed.**
  - Where: `crates/infra-postgres/migrations/20270207000000_flow_control_rate_key_listing.up.sql:61-69`.
  - No test removes a waiter while another waiter of the same key remains. There is also no rate counterpart of the concurrency side's consistency check for this column (`consistency.rs:144-233`); that work is T022-079, still open.
  - Mutation that survives: replace the `COALESCE((SELECT min…), w.oldest_wait_at)` branch with `w.oldest_wait_at`. The value would never move forward, and the CHECK would still hold. Replacing it with `NULL` would probably be caught by the queue suites, because it breaks the CHECK.
  - Effect: the listing order and `oldestWaitSince` would disagree with a replay of the waiter rows (checklist item 4).
  - Scenario: three waiters at e1 < e2 < e3 on a `1/second` key. After the slot ages out, grant one waiter (with `grant_queued_step`, or by driving the scheduler's grant pass). The listing's `oldestWaitSince` should then equal e2 exactly.
  - Fix: add that test. Also add `oldest_wait_at IS NOT DISTINCT FROM (SELECT min(enqueued_at) …)` per window to the end-of-suite consistency check, and test one statement that deletes several waiters of one key.
  - Confidence: confirmed in code (the mutation itself was not run).

**Minor**

- **Minor 1. The null estimate for a run that also waits for a concurrency permit is untested.**
  - Where: `crates/application/src/runs.rs:2374-2380`.
  - T022-078's evidence line asks that the estimate be absent in the D022-014 cases, and this is one of them. Deleting the block passes every test.
  - Fix: a start that declares both `concurrency: 1` and `rateLimit: 1/hour`. The third run waits on both gates: it should have a position and `estimatedGrantAt: null`.
  - Confidence: confirmed in code.

- **Minor 2. A run that the grant pass will not grant still gets an estimate.**
  - Where: `flow_control.rs:962-983`.
  - The query filters the waiters ahead, but not the reading run itself. If the run is paused, or its own `deadline_at <= t`, it still gets `estimatedGrantAt = t` ("room now"). An estimate later than its own `queueDeadline` is also shown, although the run will time out first.
  - The spec (FR-022-048) says the estimate is "absent when it cannot be computed".
  - Fix: return `me.deadline_at` and the run's `paused` flag. Return null when the run is paused, when `deadline_at <= t`, or when the estimate is at or after `deadline_at`.
  - Confidence: confirmed in code. Confirm with a test that pauses a waiting run and reads it.

- **Minor 3. `estimatedGrantAt` can link runs for a reader without the reveal grant.**
  - Where: `runs.rs:2365-2381`, `crates/api-http/src/dto_flow_control.rs:126`.
  - The estimate equals another run's grant instant plus `per`, to the microsecond, and it is shown to any reader of the run. That reader can compare `estimatedGrantAt - per` with another run's `rateLimit.since` and learn that the two runs share a key. The label is hidden precisely to prevent this (G022-014).
  - The listing truncates the instants derived from runs to the minute for such readers (D022-067 item 6); this field is not truncated.
  - Fix: apply `to_the_minute` (or a coarser rounding) when the reader may not reveal personal data, or have the security reviewer accept this explicitly.
  - Confidence: the missing truncation is confirmed in code; the linking risk is plausible.

- **Minor 4. Most paths of the listing are never run by a test.**
  - Untested:
    - a valid `?group=`: `RATE_KEY_LISTING_OF_GROUP_SQL` (`flow_control.rs:948`) never executes;
    - another group's cursor, which should be a 400;
    - a cursor sealed under a retired key version, which should be a 400;
    - paging after a key nobody waits for (the `'infinity'` case), and ties between such keys ordered by pool, scope, group and digest;
    - a STEP-scope window (`unleasedGrants > 0`, which should make `nextSlotAt` null);
    - 404 for another tenant's namespace, and 403 without `ops:read`.
  - No contract test checks these responses against the `additionalProperties: false` schemas.
  - Fix: one test with three or more keys, two of them with nobody waiting, paged with `limit=1`, plus the group and STEP cases.
  - Confidence: confirmed (`rate_rest.rs` is the only test file that names the route).

- **Minor 5. A test comment claims a case the test does not run.**
  - Where: `tests/integration/flow-control-step-policies/rate_rest.rs:186-190`.
  - The comment says the test checks the estimate one microsecond before the slot ages out. The test only reads after the slot has aged out.
  - Fix: remove the claim. The "estimate off by a microsecond" mutation is caught by `a_run_shows_its_rate_slot_and_a_waiting_run_its_place`, not by this test.
  - Confidence: confirmed.

- **Minor 6. The listing may not show exactly the rows the ceiling counts.**
  - The contract says the listing shows "the rows the `rate_limit_live_keys` ceiling counts". According to data-model D022-066, the ceiling counts rows under the digest versions in force, while the quota usage counts every version.
  - The listing query (`flow_control.rs:937-946`) has no `key_version` filter. After a rotation it therefore shows rows the ceiling does not count.
  - Fix: either filter by the versions in force, or change the contract text to say it matches the usage.
  - Confidence: plausible. I did not read the counting statement itself.

**Nit**

- **Nit 1.** `nextSlotAt` has a third null case the contract does not describe: somebody waits, no grant is unleased, and no grant counts for any waiter (there is room, and the pass is behind) (`flow_control.rs:930`).
- **Nit 2.** In `getRun`, `WAITING` comes from the `RUN_SELECT` statement and the position from a second statement at a later instant. A run granted in between is shown `WAITING` with `position: null`, although the contract says position is shown while WAITING in getRun.
- **Nit 3.** `rate_place` runs without the 2 s `statement_timeout` the listing uses. Its cost is bounded (at most 10,000 grants and 10,000 waiters per key), but it is the only read here without a timeout.
- **Nit 4.** A CHECK added `NOT VALID` is still enforced on rows that are updated. Between the two migrations, a window whose `waiters` counter has drifted from its rows would have its `next_eligible_at` update refused. This matters only if the counter has already drifted, and the table is empty in released deployments.

**Checked and sound**

- **Half-open windows.**
  - `Run.rateLimit` GRANTED uses `leased_at > now() - per`.
  - `grantsInWindow` is the unleased grants plus `leased_at > now() - max_period_ms`.
  - `nextSlotAt` is `min(lease + p)` over grants with `lease + p > now()`.
  - The place uses `leased_at > t - p`.
  - All four match `admits`, which uses its decision instant minus the period. At exactly lease + per a grant no longer counts in every one of them.
- **Estimate.**
  - `k = n + a - L + 1`. The sum `n + a` stays the same when a waiter ahead is granted and falls only when a counted grant ages out, so the instant is the k-th lease plus `p`. I checked this with n = 3, L = 3, a = 2.
  - `OFFSET greatest(n + a - L, 0)` is k − 1, and ties are handled correctly.
  - k ≤ 0 gives `t`, and k > n gives `None`.
  - At RUN scope no grant can be unleased, because the CHECK `rate_limit_grants_shape` requires `leased_at = granted_at`. So `n` and the set of grants searched for the k-th lease are the same.
- **Position.** It uses the same filter and order as `gate_key_waiters` (`maintenance.rs:222-227`). Leaving out `declared_limit > n` gives the same decision when a key has a single limit.
- **Microseconds and instants.**
  - Conversions are exact: PostgreSQL microseconds to `OffsetDateTime` and back, `make_interval` of 1, 60 or 3600 seconds, and `Duration::from_millis`.
  - The cursor's microsecond value round-trips exactly, and `to_the_minute` changes only the view, never the sealed position.
  - `rate_place` is one autocommit statement, so it reads at a single instant. The listing reads one REPEATABLE READ snapshot. Its `now()` is the transaction start, slightly before the snapshot, which does no harm.
- **Trigger.**
  - The grant branch keeps `live_grants`, `grants_total`, `max_period_ms` and `updated_at`. The waiter branch keeps `waiters`, `max_period_ms` on insert and `updated_at`. That is what the old single UPDATE did, since it added 0 to the other counters.
  - Waiter rows are never updated (`rate_limit_rows_guard`), so insert and delete are the only changes.
  - The CHECK holds after every trigger when one statement removes all waiters of a key, removes some, or adds and removes in either firing order.
  - The `FOR UPDATE` in the trigger adds no new lock order: every waiter writer already holds the window row `FOR UPDATE` (`gate_queue.rs` `locked_queue`, `readiness_gate.rs` `apply_returns`).
  - The approach is the same as `concurrency_semaphore_count`.
- **Down migration.** It is word for word the function of `20270130000100`, the only earlier definition. The drops run in a valid order, and the `undo` tests in `migrations.rs` pass through it.
- **Paging.**
  - The row comparison has the same columns as the ORDER BY and the two indexes.
  - The `-infinity` / `infinity` starting points are right.
  - Within one group the group listing's order is the full order.
  - The `r1` / `k1` tags keep the two listings' cursors apart.
- **Lateral.** It prefers `GRANTED` ('G' sorts before 'W') and probes the primary keys only. A state that does not match its deadline (GRANTED with a deadline, WAITING without one) is reported as corrupt.
- **Response bodies.** The DTOs carry exactly the required properties and nullable fields of `RunRateLimit`, `RateLimitKey` and `RateLimitKeyPage`.
- **Reported mutations.** Reading the five tests, each of the five hand mutations you ran would make a named test fail.
