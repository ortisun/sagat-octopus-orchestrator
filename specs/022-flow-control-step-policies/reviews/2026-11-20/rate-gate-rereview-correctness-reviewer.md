# Re-review of the rate gate fixes (feature 022): a67ba790 (code) and a3a2d2d3 (documents)

Reviewer: `correctness-reviewer` (read-only, diff and spec only), 2026-11-20, over commits a3a2d2d3 and a67ba790 of
`feature/022-throughput-control` (diff `d5e0c792..a67ba790`). Text as returned; resolutions in
`rate-gate-rereview-responses.md`.

I read the code but ran nothing. Each finding names the command that would confirm it.

**Summary:** three of the earlier Blockers and Majors are open, all left open on purpose. Two of the fixes are only partly done. The other fixes are in the code. The fixes themselves add two new Majors:
- **Lost wake-up:** a hint written with `least()` can still be lost in a race with the pass that rests the key.
- **Unbounded rows:** the live-keys fix removed the only limit on how many rate-limit rows a namespace can hold.

The reset refusal is still a retryable 503, and a pending reset still loops forever.

## Status of the earlier findings

| Finding | Status | Where |
|---|---|---|
| Correctness Major 1 / architecture Blocker 1 (checker test file) | Resolved. There is one `EveryRunIsCreatedWithACheckedInput` (line 809), the stale test is gone and a new class replaces it. The `if __name__` at line 620 was already in the base file. | `scripts/tests/test_check_architecture.py` |
| Architecture Blocker 2 (upgrade oracles) | Resolved | `release_upgrade.rs:205`, `release_upgrade_inflight.rs:181`, `release_upgrade_callback_cutover.rs:123` |
| Architecture Blocker 3 (hot-path budget) | Resolved: the budget was raised, with reasons, in the plan table and in `work_budget.rs` (60, 44, 48, 48) | `work_budget.rs` |
| Correctness Major 2 (count instant) | Resolved in code. **No test fails if it is reverted.** | `gate_queue.rs:138,163,174,180,201`; `readiness_gate.rs:345,668,683` |
| Correctness Major 3 (rest computed later than the counts) | Resolved for the race as stated. It introduces a lag (Minor 1). | `maintenance.rs:263-281` |
| Correctness Major 4 (grant through another key's pass) | **Partly resolved**: both grant paths now set the hint, but a race remains (new Major 1) | `gate_queue.rs:286-298`, `readiness_gate.rs:750-763,1436-1469` |
| Correctness Major 5 / architecture Major 1 (reset of a rate-limited start) | Open on purpose. Whether the interim behaviour is safe is judged below. | `readiness_gate.rs:1119-1127` |
| Security Major 1 (live windows) | Resolved for admission. It introduces new Major 2. The usage endpoint still counts every key version (Minor 4). | `readiness_gate.rs:619-627`, `quotas.rs:1371-1384` |
| Security Major 2 (cost of the lease pass) | **Partly resolved.** `st.attempt >= u.attempt` now gates the journal probes. Each tick still does one `steps` probe per unleased grant before `ORDER BY … LIMIT`. | `maintenance.rs` (`record_rate_leases`) |
| Architecture Major 2 (purge as of a chosen instant) | Resolved by a checker rule plus its test (one small loophole, see Nits) | `check-architecture.py` `PURGE_AS_OF_DOOR` |
| Architecture Major 3 (consistency check of the rate tables) | Open. T022-079 is not ticked. **It is not in the runbook's production gate list** (Minor 7). | — |
| Architecture Major 4 (lock-order barrier tests) | Open, and the runbook §6 gate for it is honest. None of the fixes adds a new lock order: the hint after a grant writes a window the caller already holds, the lease pass's lowering locks one window and holds no grant row, and the purge locks the window, then its grants, with `SKIP LOCKED`. | — |
| Architecture Major 5 (purge does not scale) | **Partly resolved**: `SKIP LOCKED`, at most 500 per key, and the `rate_purge` counter. There is still no order, no index, no gauge, and the early exit of Minor 3. | `maintenance.rs:1305-1390`, `passes.rs:4296-4321` |
| Architecture Major 6 / correctness minor 1 (refusal kind) | Resolved in code. Only the rate side is tested end to end (Minor 6). | `readiness_gate.rs:482,1573-1590` |

### Is the reset gate honest, and is the interim behaviour safe?

**The refusal is not terminal. It is still a retryable 503:**
- `readiness_gate.rs:1120-1127` returns `BinaryCapabilityMissing`, and `crates/api-http/src/error.rs:1220` turns it into `unavailable()`. A synchronous reset is refused 503, retryable, and every process of this build refuses it the same way.
- `reset_completion.rs:241-243` maps the refusal to `None`, whose comment says another process completes the entry. That is false here: no process does.
- `:380-386` then backs off and retries. A pending reset never resolves and keeps its `pending_resets` quota.

**The gate is not recorded consistently:**
- D022-065 item 7 and the responses call it a "release gate".
- The G022-016 row in the gaps table says only "Open; T022-086". Compare G022-015, which says "the release cannot close with it open".
- Only runbook §6 forbids production activation, and nothing in the code enforces it.

The interim behaviour is therefore unsafe wherever `rate-limits` is activated, staging included.

**Smallest fix:**
1. Give this case its own non-retryable refusal and make `failure_of` answer `Some("ADMISSION_REFUSED")` for it; or refuse the reset at request validation with a 422.
2. Mark G022-016 as a release gate in the gaps table.
3. Add tests for the synchronous and the pending paths.

**Confirm:** a test that resets a rate-limited run through the API and through the completer, and asserts the status and the entry's state. There is none today.

## Blocker

None.

## Major

### Major 1. A hint written by `least()` can be lost: a concurrent rest still matches and computes from an old snapshot (plausible; the mechanism is confirmed in code)

**Where:**
- `maintenance.rs:263-281`: the rest's compare-and-set `w.next_eligible_at = $5` plus a subquery over `rate_limit_grants`.
- `readiness_gate.rs:1447-1457` (`window_due_after_grant`) and `maintenance.rs:367-389` (the lease pass's lowering). Both write `least(existing, …)`.

**Mechanism:**
- When the window's hint `H` is already at or before now, `least(H, now + p)` writes `H` back unchanged.
- Under READ COMMITTED, a rest UPDATE that blocked on the window row re-checks its WHERE on the new row version. The compare-and-set still matches.
- Its SET subquery, however, runs with the statement's original snapshot. That is PostgreSQL's documented behaviour ("does not see effects of those commands on other rows").
- So the new grant is invisible to the rest. If no other grant counts, the rest writes `NULL`.

**Scenario** (correctness Major 4's setup, run scope):
1. Window `k` holds no counted grant. Its hint `H` came due because a third waiter queued.
2. The window's pass refuses the head (dual-gated, its permit is held), then refuses `behind` (the head's reservation).
3. The permit is released. The semaphore's pass locks `k` and grants the head both gates with a run grant at `t`. `window_due_after_grant` writes `least(H, t + 60s) = H`.
4. The window's rest statement started before step 3 committed. It blocked on `k`, re-checked `= H` (true), computed `NULL`, and committed.
5. The head's slot frees at `t + 60s`. Nothing wakes `behind`, which fails `RATE_LIMIT_WAIT_TIMEOUT`.

At step scope the lease pass's lowering rescues the window once the head is leased. At run scope nothing does. The same race can also drop a lease lowering (Minor 2).

**Why the tests miss it:** `rate_gate::a_grant_through_another_keys_pass_wakes_the_waiters_of_its_window` runs the two passes in sequence, never concurrently.

**Fix:**
- Preferred: compute the rest after taking the lock, in one transaction: `SELECT … FOR UPDATE` with the compare-and-set, then the UPDATE as a second statement, whose new snapshot sees every grant committed under the lock.
- Alternative: compare on `updated_at` as well. The triggers and both `least()` writers already bump it.

**Confirm:** a barrier test, T022-081 style (it does not exist yet).
- Session A runs `BEGIN; SELECT … FROM rate_limit_windows … FOR UPDATE`, then inserts a run grant.
- Session B calls `rest_gate_key` and blocks.
- A runs `window_due_after_grant` and commits.
- Assert that the hint is not `NULL`.
- Run with `cargo test -p sagat-tests --test integration_flow_control_step_policies rate_limit_locks`.

### Major 2. Counting only live windows removed the only bound on rate-limit rows (confirmed in code; how fast rows grow depends on load)

**Where:** `readiness_gate.rs:619-627` together with the purge's fixed capacity:
- `passes.rs:93` and `:4296-4321`: 64 keys × 20 rounds = 1,280 keys per housekeeping per replica.
- `maintenance.rs:1312-1320`: an unordered, unindexed `DISTINCT … LIMIT`.

**Mechanism:**
- Before the fix, every window row counted against `rate_limit_live_keys`, so a namespace held at most about 10,000 rows (G022-002 aside).
- Now a per-second window stops counting one second after its last lease. The purge keeps grants for `max_period + 60s`, and only then can the window row go.

**Scenario:**
- A definition with `rateLimit: {key: input.requestId, limit: 1, per: second}` and 100 starts a second.
- The live count is about 100, never refused.
- 100 windows and 100 grants are created each second. The purge removes at most about 4.3 keys a second per replica (1,280 / 300 s).
- The backlog grows by about 95 rows a second, roughly 8 million a day, in tables every tenant shares:
  - The purge scans them in full up to 20 times per housekeeping on every replica.
  - `live_keys` runs a correlated `EXISTS` for every window of the namespace on each new-key admission, so this tenant's admission cost grows with the backlog.

G022-018 records the purge backlog but says live-key counting "no longer depends on it". It does not say that nothing bounds the rows any more.

**Why the tests miss it:** no test creates more keys than the purge visits.

**Fix:**
- Keep the live count for admission, which is what security Major 1 asked for.
- Add a second, hard limit on the namespace's total window rows (for example 2 × the ceiling), so the backlog is bounded again.
- Or have an admitting transaction purge a bounded number of its own namespace's aged windows.
- In either case, give the purge an index on `leased_at` and an oldest-first order.

**Confirm:** a test that starts 2,000 distinct per-second keys under `rate_limit_live_keys = 1,000`, waits on the database clock, runs one housekeeping, and asserts the row count. Today the count would be about 2,000 rows and no refusal.

## Minor

1. **The rest is anchored on the hint it was selected by, so the hint can fall behind real time** (`maintenance.rs:266-276`).
   - The new hint is the next grant age-out after `H`, which may already be past. Each visit therefore moves the hint forward by one grant.
   - A key that grants more than one slot per tick keeps its hint up to `max_period + 60s` in the past. `due_gate_keys` orders by `next_eligible_at` (`maintenance.rs:147-156`), so lagging keys are served before any key that came due later.
   - Keys with 256 or more waiters already behaved this way (`passes.rs:5713` never rests them).
   - Fix: return the selection statement's `now()` as a column and rest from it. It is at or before every count the pass makes and at or after `H`.
   - `rate_gate.rs:455` pins the current value and would follow.
2. **The lease pass's lowering is not guaranteed.**
   - It runs after the commit, one window per statement (`maintenance.rs:351-389`). A crash in between, or the race of Major 1, leaves the hint up to `H − L` late, where `L` is the lease instant.
   - Runbook §6 says "visited at the latest one copy after its slot frees", which can then be false.
   - **No test fails if the lowering loop is deleted.** `rate_gate.rs:430` and `:552` record the lease before the pass, so the rest alone gives the same hint.
   - Add a test: lease, run `grant_gates`, then `record_rate_leases`, and assert the hint is `L + 60s`.
3. **The purge stops at the first round that deletes nothing** (`passes.rs:4304`).
   - With `SKIP LOCKED` (`maintenance.rs:1341`) and an unordered `LIMIT 64`, a round whose candidates are all locked, or already purged by another replica, returns 0 and ends the housekeeping's purge. The same candidates tend to come back next time.
   - Fix: break when the candidate list is empty, not when nothing was deleted; order the candidates.
4. **The usage still counts every digest version** (`quotas.rs:1371-1384`; its own comment says the gate leaves out forcibly retired rows). The "nit 7 fixed" claim is only partly true.
5. **`verify_start_rate`'s invariant 10 reads the journal, which still holds purged grants** (`execution.rs:888-892`).
   - So it reports a breach for the behaviour G022-017 accepts: a later hourly acquirer admitted after the purge.
   - It also has no namespace filter (a 48-bit label collision would be a false positive).
   - Correctness minor 4's point about G022-017 was not answered. T022-080 will trip on it.
6. **Tests that would not fail if a fix were reverted:**
   - Correctness Major 2: none.
   - Security Major 1: no test that an aged window admits a new key before the purge.
   - The refusal kind's call site (`readiness_gate.rs:482`): a constant `Some(GateKind::RateLimit)` passes both the unit test (which passes `key_of` directly) and `rate_gate.rs:296`.
   - The `if waiting` hint in `decide_filed` (`readiness_gate.rs:750`).
   - The lease filter of security Major 2.
   - Correctness Major 3 proper (the `> $5` filter): only the coalesce part is caught, by `rate_gate.rs:455`.
   - `assert_consistent` checks no step-scope rate rows, yet ticked T022-074 relies on tests that call it.
7. **The gate lists are incomplete.**
   - Runbook §6 forbids production before T022-080, 081, 086 and 104, but omits T022-079 (architecture Major 3).
   - The G022-016 row lacks "release gate" (see the reset judgement above).

## Nit

- `window_due_after_grant` and the lowering: if `waiters > 0` but the waiter subquery returns `NULL` (the counters drifted), `least(coalesce(hint, 'infinity'), NULL)` writes `'infinity'`. `key_due` and `rearm_queued_keys` only reset a `NULL` hint, so the key would sleep until a return or a timeout. Guard with `coalesce(...)` on the subquery.
- The purge rule allows any `self.purge_rate_grants_as_of(` in `maintenance.rs`, not just the call from `purge_rate_grants`.
- `if __name__` at `test_check_architecture.py:620` (already in the base file): running the file as a script skips `ThePurgeAsOfAChosenInstantIsTheTestsOnly`. CI's `discover` is unaffected.

## Checked and sound

- **The fixes' arithmetic is consistent, half-open throughout:**
  - The count is `leased_at > now − p`.
  - The rest uses `leased + p > H`, its hint `leased + p`, and the key is due at `<= now()`.
  - The hint after a grant is `now + p_min`; the lowering is `L + p_min`.
  - The purge deletes on `<` (kept at the boundary, deleted 1 µs later, and the test checks both).
  - Verify counts `(at − p, at]`.
  - `make_interval(secs => ms/1000.0)` is exact for 1,000, 60,000 and 3,600,000 ms.
- **Clocks:**
  - Every instant comes from the database and round-trips through `OffsetDateTime` at microsecond precision.
  - No new code reads the host clock.
  - The purge's instant is read before its statements, which is conservative.
  - Run grants store `leased_at = granted_at` on both paths.
  - One instant per decision, read after the locks, holds for `admits`, `key_count` and the grant.
- **Lock order:**
  - The lease UPDATE fires only the guard trigger and never touches a window.
  - The purge takes the window, then its grants.
  - The hint after a grant writes a window the caller already holds.
- **Other changes:**
  - The negative test for `verify_run` would fail if the call to `verify_start_rate` were removed.
  - The checker rule has a test that fails on a production call.
  - The three upgrade oracles and the budget lines agree with the plan table.
