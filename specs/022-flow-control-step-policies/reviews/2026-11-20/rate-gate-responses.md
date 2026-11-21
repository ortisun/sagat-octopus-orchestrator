# Responses to the reviews of the rate gate (2026-11-20)

Reviews: `rate-gate-correctness-reviewer.md`, `rate-gate-security-reviewer.md`, `rate-gate-architecture-reviewer.md`.
Decisions: decision D022-064 (as revised) and D022-065 in `../../research.md`. Each finding was checked against the code.

| Finding | Resolution |
|---|---|
| Architecture Blocker 1, correctness Major 1, security minor 5 (checker test file garbled) | Fixed: the file restored and only the stale test removed (the slicing of the edit took the first `if __name__` of the file). `python3 -m unittest scripts/tests/test_check_architecture.py` |
| Architecture Blocker 2 (upgrade oracles) | Fixed: the three oracles assert `20270206000100`; `scripts/check-upgrade-boundary.py` passes |
| Architecture Blocker 3 (hot-path budget) | Measured red, then raised with its reason: the rate-limited next step 59 → 60 (the window's existence, T022-077), the rate-limited entry 42 → 44 and the gated start 47 → 48: one statement each is the fleet activation read added by commit 295b158b (found by diffing the statement lists of commits 64b6f2d9 and 34b8be41), the other the window's existence; written first in the plan's budget table |
| Correctness Major 2 (count instant) | Fixed (D022-065 item 1): every count binds the decision's instant |
| Correctness Major 3 (rest later than counts) | Fixed (D022-065 item 2): the rest uses the hint the pass was selected by; `rate_gate::an_unleased_slot_rests_its_key_until_the_pass_start_plus_the_window` checks it exactly |
| Correctness Major 4 (grant through another key's pass) | Fixed (D022-065 item 2): `window_due_after_grant`; `rate_gate::a_grant_through_another_keys_pass_wakes_the_waiters_of_its_window` (the window had no hint before) |
| Correctness minor 2, architecture on decision D022-064 item 5 (lease lag) | Fixed: the lease pass lowers each recorded window's hint after its commit, one window a statement; "exact" withdrawn |
| Correctness Major 5, architecture Major 1 (reset of a rate-limited start) | Kept with a release gate (D022-065 item 7, G022-016): T022-086; runbook §6 forbids production activation before it |
| Correctness minor 1, architecture Major 6 (refusal kind) | Fixed (D022-065 item 3); the unit test now covers both dual-gated cases |
| Security Major 1 (live windows and purge lag) | Fixed (D022-065 item 4) in the gate and the usage |
| Security Major 2 (lease pass cost) | Fixed: the steps projection filters before the journal probe; index change left with G022-018 |
| Security minor 6, architecture Major 5 (purge) | Partly: `SKIP LOCKED`, 500 a key, a metric; shard-driven purge is G022-018 |
| Architecture Major 2, security minor 4 (test door) | Guarded by a checker rule (`ThePurgeAsOfAChosenInstantIsTheTestsOnly`) |
| Security minor 3, architecture minor 5 (runbook) | Fixed: §1.1, §2 step 3 and §6 rewritten |
| Architecture Major 3, Major 4 (consistency check, barriers) | Open: T022-079 (`key_consistency` for rate tables) and T022-081 |
| Correctness minor 3 (`hint_us`) | Fixed: the window is the waiter's |
| Correctness minor 4 (verify negative test) | Added `rate_gate::verify_run_refuses_a_start_whose_grant_row_is_gone_while_it_still_counts`; `grants_total` and the purged branch stay T022-079 |
| Correctness minor 5 (vacuous and timing assertions) | Removed both |
| Correctness minor 6, architecture minor 10 (creators, release, dual queue) | Open: T022-072 is not ticked; its remaining evidence is listed under it |
| Architecture minor 1 (lease UPDATE lock order) | Accepted: overlapping lease passes happen only in a takeover; a deadlock aborts one statement, retried next tick |
| Architecture minor 2, minor 3, minor 4 | Fixed: `outcome="failed"`, doc comments, the trait's bound |
| Architecture minor 6, minor 7, minor 8, minor 9 | Open: the down's refusal test, 041's list restatement at merge time (named in the migration header), the ADR-0088 note, the per-second clamp shown in T022-078 |
| Security suspicions 1 to 4, nits 7 and 8 | Suspicion 1 left to task T022-079's consistency check; suspicion 2 bounded by the lease transaction's length (recorded); suspicion 3 is gap G022-014's class; suspicion 4 noted for the owner report; nit 7 fixed with item 4; nit 8 follows the existing convention |
